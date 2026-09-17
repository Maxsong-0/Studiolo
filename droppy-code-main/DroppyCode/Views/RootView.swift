import AppKit
import SwiftUI

struct RootView: View {
    @Environment(AppModel.self) private var model
    @State private var detailSize: CGSize = .zero

    var body: some View {
        let sidebar = model.sidebar
        // With the sidebar floating, the collapsed column hands its list to a panel over
        // the chat rather than the toolbar popover; only-floating drops the column altogether.
        let onlyFloats = model.settings.sidebarFloats && model.settings.sidebarOnlyFloats
        let floats = model.settings.sidebarFloats && (onlyFloats || !sidebar.isVisible)
        HStack(spacing: 0) {
            SidebarView()
                .frame(width: onlyFloats ? 0 : sidebar.renderedWidth)
                .clipped()
                .onGeometryChange(for: CGFloat.self, of: { $0.size.width }) { sidebar.noteLaidOutWidth($0) }
                .overlay(alignment: .trailing) {
                    SidebarResizeHandle(
                        isActive: !onlyFloats && sidebar.isVisible,
                        onBegin: { sidebar.beginDrag() },
                        onChange: { sidebar.drag(by: $0) },
                        onEnd: { sidebar.endDrag() }
                    )
                    .frame(width: SidebarResizeHandle.hitWidth)
                    .padding(.top, Chrome.trafficLightTop + Chrome.trafficLightDiameter)
                }

            DetailView()
                .frame(minWidth: 0, maxWidth: .infinity)
                // The sheet keeps to its own bounds. A conversation gliding sideways to make
                // room for a panel (`ReserveSlide` carries it on an offset), a panel settling
                // into a corner of a pane that just changed size, and rows re-wrapping in a
                // live resize all reach past the sheet for a few frames, and drew over the
                // sidebar or off the window's edge until they landed.
                .clipped()
                .padding(.leading, sidebar.isVisible && !onlyFloats ? 0 : Chrome.sheetInset)
                .padding(.trailing, Chrome.sheetInset)
                .padding(.vertical, Chrome.sheetInset)
                .onGeometryChange(for: CGSize.self, of: { $0.size }) { detailSize = $0 }
                .overlay(alignment: .topLeading) {
                    if floats, detailSize != .zero {
                        FloatingSidebarPanel(area: detailSize, canClose: !onlyFloats) { sidebar.toggle() }
                            .transition(.scale(scale: 0.94).combined(with: .opacity))
                    }
                }
                .animation(Chrome.panelSlide, value: floats)
        }
        // While the sidebar is hidden, dragging the window's leading edge pulls it back out.
        .overlay(alignment: .leading) {
            SidebarResizeHandle(
                isActive: !onlyFloats && !sidebar.isVisible,
                onBegin: { sidebar.beginDrag() },
                onChange: { sidebar.drag(by: $0) },
                onEnd: { sidebar.endDrag() }
            )
            .frame(width: SidebarResizeHandle.hitWidth)
            .padding(.top, Chrome.trafficLightTop + Chrome.trafficLightDiameter)
        }
        .background { WindowBackdrop() }
        .background { WindowChromeConfigurator(sidebarVisible: sidebar.holdsTrafficLights, sidebarDragging: sidebar.isDragging) }
        // Clipped after the backdrop is painted, so the glass and its edge stop at the window's own curve.
        .clipShape(RoundedRectangle(cornerRadius: Chrome.windowCornerRadius, style: .continuous))
        .ignoresSafeArea()
        .coordinateSpace(.named(GenieAnimator.coordinateSpace))
        .overlay { GenieLayer() }
        .overlay { RowGlideLayer() }
        .overlay {
            if model.isCommandPalettePresented {
                CommandPalette()
            }
        }
        .dropDestination(for: URL.self) { urls, _ in
            handleDrop(urls)
        }
        .alert(
            model.alert?.title ?? "",
            isPresented: Binding(
                get: { model.alert != nil },
                set: { if !$0 { model.alert = nil } }
            ),
            presenting: model.alert
        ) { _ in
            Button("OK", role: .cancel) {}
        } message: { alert in
            Text(alert.message)
        }
    }

    @discardableResult
    private func handleDrop(_ urls: [URL]) -> Bool {
        var folders: [URL] = []
        var files: [URL] = []
        for url in urls {
            var isDirectory: ObjCBool = false
            guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory) else { continue }
            if isDirectory.boolValue { folders.append(url) } else { files.append(url) }
        }
        var lastProject: Project?
        for folder in folders { lastProject = model.addProject(at: folder) }
        if let lastProject { model.newThread(in: lastProject) }
        if !files.isEmpty, let threadID = model.selectedThreadID {
            Storage.attach(files.map(AttachmentSource.file), to: Bindable(model.runtime(for: threadID)).draft.attachments)
        }
        return !folders.isEmpty || !files.isEmpty
    }
}

struct DetailView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let threadID = model.selectedThreadID, model.thread(threadID) != nil {
            ChatView(runtime: model.runtime(for: threadID))
        } else {
            Group {
                if model.projects.isEmpty {
                    WelcomeView()
                } else {
                    NoThreadView()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .detailSheet()
        }
    }
}

struct WelcomeView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        ScrollView {
            VStack(spacing: 30) {
                VStack(spacing: 14) {
                    Image(nsImage: NSApp.applicationIconImage)
                        .resizable()
                        .frame(width: 104, height: 104)
                    Text("Droppy Code")
                        .font(.system(size: 34, weight: .semibold))
                    Text("The coding app by Droppy. A calm, native home for your coding agents.")
                        .font(.system(size: 15))
                        .foregroundStyle(Chrome.secondaryText)
                }
                VStack(spacing: 10) {
                    Button {
                        model.chooseProjectFolder()
                    } label: {
                        Label("Add a project", systemImage: "folder.badge.plus")
                            .padding(.horizontal, 6)
                    }
                    .buttonStyle(.glassProminent)
                    .controlSize(.extraLarge)
                    Text("Or drop a folder anywhere in this window.")
                        .font(.system(size: 12))
                        .foregroundStyle(Chrome.secondaryText)
                }
                ChromeSection(title: "Providers") {
                    ProviderChecklist()
                }
                .frame(maxWidth: 480)
            }
            .padding(48)
            .frame(maxWidth: .infinity)
        }
        .scrollIndicators(.never)
        .scrollBounceBehavior(.basedOnSize)
    }
}

struct ProviderChecklist: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        ChromeCard {
            ForEach(Array(ProviderKind.allCases.enumerated()), id: \.element) { index, provider in
                if index > 0 { ChromeRowDivider() }
                ProviderStatusRow(provider: provider)
            }
        }
    }
}

private struct ProviderStatusRow: View {
    @Environment(AppModel.self) private var model
    let provider: ProviderKind

    var body: some View {
        let status = model.providers.status(provider)
        HStack(spacing: 12) {
            ProviderIcon(provider: provider, size: 18)
                .foregroundStyle(Chrome.primaryText)
                .frame(width: 22)
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: provider.displayName)
                    .font(.system(size: 13))
                Text(verbatim: status.isChecking ? "Checking…" : status.summary)
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
            }
            Spacer()
            if status.isChecking {
                ProgressView()
                    .controlSize(.small)
            } else if !status.isInstalled {
                if provider.isAPIKeyBased {
                    // Nothing to install: the key is typed in Settings, so the row opens it
                    // rather than sending the reader to a web page for it.
                    OpenProvidersSettingsButton()
                } else {
                    Link("Install", destination: provider.installURL)
                        .buttonStyle(.glass)
                }
            } else if status.auth == .signedOut {
                if provider.isAPIKeyBased {
                    OpenProvidersSettingsButton()
                } else {
                    CopyCommandButton(command: provider.loginCommand)
                }
            } else {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(Chrome.success)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 11)
    }
}

/// Takes the reader to the Providers page, where a key-based provider's key is typed.
private struct OpenProvidersSettingsButton: View {
    var body: some View {
        Button("Add key") {
            WindowManager.shared.showSettings()
            SettingsNavigation.shared.requestedPage = .providers
        }
        .buttonStyle(.glass)
        .help("Open Settings > Providers")
    }
}

struct CopyCommandButton: View {
    let command: String
    @State private var didCopy = false

    var body: some View {
        Button {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(command, forType: .string)
            didCopy = true
            // Back to the command after a beat, the way the transcript's copy button
            // goes back to its mark; it used to read "Copied" for the rest of the launch.
            Task {
                try? await Task.sleep(for: .seconds(1.4))
                didCopy = false
            }
        } label: {
            Label(didCopy ? "Copied" : command, systemImage: didCopy ? "checkmark" : "terminal")
                .font(.system(size: 11, design: .monospaced))
        }
        .buttonStyle(.glass)
        .help("Copy the sign-in command, then run it in Terminal")
    }
}

struct NoThreadView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: "bubble.left.and.text.bubble.right")
                .font(.system(size: 46, weight: .light))
                .foregroundStyle(Chrome.secondaryText)
            Text("Pick a thread or start a new one")
                .font(.system(size: 17, weight: .medium))
            Button {
                model.newThread()
            } label: {
                Label("New thread", systemImage: "square.and.pencil")
            }
            .buttonStyle(.glassProminent)
            .controlSize(.large)
            // The chord as the store has it now, so a remap never leaves a stale key here.
            if let chord = ShortcutStore.label(for: .newThread) {
                Text(verbatim: chord)
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
