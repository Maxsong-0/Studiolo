// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// The shape shared by the prompt editors (a queued follow-up, a sent message): a title,
/// the text, its files with a way to add more, and a row of buttons. Command-Return submits
/// and Return adds a line, as in the chat box, and Escape cancels; what those do, and what
/// sits between the files and the buttons, is the caller's.
struct PromptEditor<Extra: View, Actions: View>: View {
    let title: String
    @Binding var text: String
    @Binding var attachments: [Attachment]
    let onSubmit: () -> Void
    let onCancel: () -> Void
    @ViewBuilder var extra: () -> Extra
    @ViewBuilder var actions: () -> Actions

    @State private var textHeight: CGFloat
    @State private var controller = ComposerController()
    @State private var showingFiles = false

    init(
        title: String,
        text: Binding<String>,
        attachments: Binding<[Attachment]>,
        onSubmit: @escaping () -> Void,
        onCancel: @escaping () -> Void,
        @ViewBuilder extra: @escaping () -> Extra,
        @ViewBuilder actions: @escaping () -> Actions
    ) {
        self.title = title
        _text = text
        _attachments = attachments
        self.onSubmit = onSubmit
        self.onCancel = onCancel
        self.extra = extra
        self.actions = actions
        // Sized for its text from the first layout: the text view reports its height a
        // pass later, and a popover that opened one line tall and then grew to three
        // played that growth over its own appearance.
        _textHeight = State(initialValue: Self.measuredHeight(of: text.wrappedValue))
    }

    /// The text view's height for `text` at the editor's width, the way the view measures
    /// itself (`ComposerTextView.Coordinator.updateHeight`): the used rect plus the container inset.
    private static func measuredHeight(of text: String) -> CGFloat {
        // 520 wide, less the outer and the field's own padding (the container has no fragment padding).
        let width = editorWidth - 2 * 20 - 2 * 10
        let bounds = (text.isEmpty ? " " : text as NSString).boundingRect(
            with: NSSize(width: width, height: .greatestFiniteMagnitude),
            options: [.usesLineFragmentOrigin, .usesFontLeading],
            attributes: [.font: NSFont.systemFont(ofSize: 14)]
        )
        return ceil(max(18, bounds.height) + 2 * 2)
    }

    private static var editorWidth: CGFloat { 520 }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(verbatim: title)
                .font(.system(size: 17, weight: .semibold))
            ComposerTextView(
                text: $text,
                height: $textHeight,
                placeholder: "Message",
                controller: controller,
                onKey: handleKey,
                onAttach: attach,
                onCursorChange: { _ in }
            )
            .frame(height: min(240, max(22, textHeight)))
            .padding(10)
            .background(Chrome.overlay(0.05), in: .rect(cornerRadius: 12, style: .continuous))
            if !attachments.isEmpty {
                EditorAttachments(attachments: $attachments)
            }
            extra()
            HStack(spacing: 8) {
                Button {
                    showingFiles.toggle()
                } label: {
                    Image(systemName: "plus")
                }
                .buttonStyle(.glass)
                .help("Add files")
                .accessibilityLabel(Text("Add files"))
                .disabled(attachments.count >= Storage.attachmentLimit)
                .popover(isPresented: $showingFiles, arrowEdge: .bottom) {
                    DownloadsPopover(
                        pick: { attach([.file($0)]) },
                        chooseOther: { showingFiles = false; chooseFiles() }
                    )
                }
                Spacer()
                actions()
            }
            .controlSize(.regular)
        }
        .padding(20)
        .frame(width: Self.editorWidth)
    }

    private func handleKey(_ key: ComposerKey) -> Bool {
        switch key {
        // The queue chord submits too: there is no queue here, and the menu's Queue a chat
        // would otherwise take the chord for the chat box behind the editor.
        case .submit, .queue:
            onSubmit()
            return true
        case .escape:
            onCancel()
            return true
        // Nothing sits before the text here, so Delete at the start is the text view's, and
        // Return is its new line.
        case .up, .down, .tab, .newline, .deleteAtStart:
            return false
        }
    }

    private func chooseFiles() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.prompt = "Attach"
        guard panel.runModal() == .OK else { return }
        attach(panel.urls.map(AttachmentSource.file))
    }

    private func attach(_ sources: [AttachmentSource]) {
        Storage.attach(sources, to: $attachments)
    }
}

/// A prompt editor as an anchored popover instead of a modal sheet, so editing never
/// takes over the window. Application-defined so AppKit never closes it on its own: it
/// survives its nested panels (the file picker, the recent-downloads popover, an
/// attachment preview). It follows the thread's edit it is synced to: Save, Send, Cancel,
/// Escape, the pencil again and a click elsewhere in the chat window all clear that,
/// which closes it; its row going away closes the popover alone and leaves the edit for
/// the row's return.
@MainActor
final class PromptEditorPopover<Content: View>: NSObject {
    /// Made on the first show. Every row that can be edited owns one of these, and a
    /// popover per row that is never opened was an AppKit window waiting for nothing.
    private var shownPopover: NSPopover?
    private var popover: NSPopover {
        if let shownPopover { return shownPopover }
        let popover = NSPopover()
        popover.behavior = .applicationDefined
        popover.animates = true
        shownPopover = popover
        return popover
    }
    private var isShown: Bool { shownPopover?.isShown == true }
    private var anchor: WeakView?
    /// The anchor as a rect in its window, for a view that must not mount an AppKit view
    /// (the follow-up queue tab): where it sits, reported by the view's own geometry
    /// (see `WindowRectAnchor`).
    private var anchorRect: CGRect?
    private var monitors: [Any] = []
    /// Clears the thread's edit, for the dismissals the popover sees itself.
    private var dismiss: () -> Void = {}
    /// Counts syncs, so a show deferred by one is dropped once a later sync closed it.
    private var generation = 0

    /// The pencil button's own view. Captured from the button's background, so
    /// it is always the live view.
    func setAnchor(_ view: NSView) {
        anchor = WeakView(view)
    }

    /// The anchor's frame in its window, for the follow-up queue tab's pencil.
    func setAnchor(windowRect: CGRect) {
        anchorRect = windowRect
    }

    /// Shows the popover on `edit`, or closes it when there is none. Safe to call
    /// as often as the state or the anchor changes: an open popover stays put.
    func sync(_ edit: PromptEdit?, dismiss: @escaping () -> Void, content: @escaping (PromptEdit) -> Content) {
        generation += 1
        guard let edit else {
            close()
            return
        }
        let hasAnchor = anchor?.value?.window != nil || anchorRect != nil
        guard !isShown, hasAnchor else { return }
        // Off the current pass: the anchor reports from inside a SwiftUI update, and
        // hosting the editor there would lay out a view tree inside another's update.
        let generation = generation
        Task { @MainActor [weak self] in
            guard let self, self.generation == generation else { return }
            show(edit, dismiss: dismiss, content: content)
        }
    }

    private func show(_ edit: PromptEdit, dismiss: @escaping () -> Void, content: (PromptEdit) -> Content) {
        guard !isShown else { return }
        let target: (view: NSView, rect: NSRect)
        if let anchor = anchor?.value, anchor.window != nil {
            target = (anchor, anchor.bounds)
        } else if let anchorRect, let resolved = WindowRectAnchor.target(for: anchorRect) {
            target = resolved
        } else {
            return
        }
        self.dismiss = dismiss
        let host = NSHostingController(rootView: content(edit)
            .fixedSize(horizontal: false, vertical: true)
            .onGeometryChange(for: CGSize.self) { $0.size } action: { [weak self] size in
                Task { @MainActor in self?.resize(to: size) }
            }
            .buttonBorderShape(.capsule)
            .tint(ThemeManager.spec.accent))
        host.sizingOptions = []
        popover.contentViewController = host
        popover.contentSize = host.view.fittingSize
        popover.show(relativeTo: target.rect, of: target.view, preferredEdge: .maxY)
        startMonitors()
    }

    /// The content grew or shrank (a line added, files attached). Only the size changes:
    /// a shown popover follows its content size in place, and showing it again for the
    /// new size replayed its appearance as a flash.
    private func resize(to size: CGSize) {
        guard size.width > 0, size.height > 0, let shownPopover,
              abs(shownPopover.contentSize.height - size.height) > 0.5 else { return }
        // The size follows at once, never as a morph: the popover's own animation is for
        // its appearance and its close, and a resize played through it as a wobble.
        let animates = shownPopover.animates
        shownPopover.animates = false
        shownPopover.contentSize = size
        shownPopover.animates = animates
    }

    /// Closes the popover without touching the thread's edit.
    func close() {
        stopMonitors()
        if isShown { shownPopover?.performClose(nil) }
    }

    private func startMonitors() {
        guard monitors.isEmpty else { return }
        if let monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown, handler: { [weak self] event in
            guard event.keyCode == 53 else { return event } // Escape
            self?.dismiss()
            return nil
        }) {
            monitors.append(monitor)
        }
        if let monitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown], handler: { [weak self] event in
            self?.handleMouseDown(event) ?? event
        }) {
            monitors.append(monitor)
        }
    }

    /// A click in the chat window dismisses the editor, and still reaches its
    /// target. The editor, the file picker and the editor's nested popovers
    /// are windows of their own, so clicks there pass, as does the pencil
    /// (which toggles on its own).
    private func handleMouseDown(_ event: NSEvent) -> NSEvent? {
        if let anchor = anchor?.value {
            guard let window = event.window, window === anchor.window else { return event }
            if anchor.bounds.contains(anchor.convert(event.locationInWindow, from: nil)) { return event }
            dismiss()
            return event
        }
        // A rect anchor (the follow-up queue tab): a click in that window dismisses the
        // editor and still reaches what it hit. A click on the anchor itself passes
        // through, so the pencil's own toggle closes the editor (see
        // `AttachmentPreviewPanel.handleMouseDown`).
        if let anchorRect, let target = WindowRectAnchor.target(for: anchorRect), event.window === target.view.window,
           !target.rect.contains(target.view.convert(event.locationInWindow, from: nil)) {
            dismiss()
        }
        return event
    }

    private func stopMonitors() {
        for monitor in monitors { NSEvent.removeMonitor(monitor) }
        monitors.removeAll()
    }
}
