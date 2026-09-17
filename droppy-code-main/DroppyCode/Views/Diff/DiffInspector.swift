import AppKit
import SwiftUI

struct DiffInspector: View {
    let runtime: ThreadRuntime

    @State private var files: [DiffFile] = []
    @State private var summary = FileChangeSummary(files: [])
    @State private var isLoading = false
    @State private var collapsed: Set<String> = []
    @State private var changedTurns: [TurnRecord] = []
    /// The file to bring into view, set when a tapped tool row focused its edits.
    @State private var scrollTarget: String?
    /// Bumped with every focus load: state can survive a re-opened popover, so the
    /// scroll is driven by a counter that always changes, never by the target alone.
    @State private var scrollRequest = 0

    private var loadKey: String {
        "\(runtime.diffRevision)-\(runtime.diffSelection?.uuidString ?? "all")-\(runtime.diffFocusRevision)"
    }

    var body: some View {
        @Bindable var runtime = runtime
        VStack(spacing: 0) {
            GlassEffectContainer(spacing: 2) {
                HStack(spacing: 10) {
                    ChromeTextMenu(
                        symbol: "plusminus",
                        title: runtime.diffSelection
                            .flatMap { id in runtime.turns.first { $0.id == id } }
                            .map { "Turn \($0.index + 1)" } ?? "All changes",
                        help: "Choose which changes to show"
                    ) {
                        PopoverItem("All changes", isChecked: runtime.diffSelection == nil) {
                            runtime.clearDiffFocus()
                            runtime.diffSelection = nil
                        }
                        ForEach(changedTurns) { turn in
                            PopoverItem("Turn \(turn.index + 1)", isChecked: runtime.diffSelection == turn.id) {
                                runtime.clearDiffFocus()
                                runtime.diffSelection = turn.id
                            }
                        }
                    }
                    Spacer(minLength: 8)
                    if !files.isEmpty {
                        Text(summary.files.count == 1 ? "1 file" : "\(summary.files.count) files")
                            .font(.system(size: 11))
                            .foregroundStyle(Chrome.secondaryText)
                        DiffStatLabel(
                            additions: summary.additions,
                            deletions: summary.deletions
                        )
                    }
                    ChromeCapsule {
                        ChromeMenuButton(symbol: "ellipsis", help: "More") {
                            PopoverItem("Expand all", symbol: "arrow.down.right.and.arrow.up.left") { collapsed.removeAll() }
                            PopoverItem("Collapse all", symbol: "arrow.up.left.and.arrow.down.right") { collapsed = Set(files.map(\.id)) }
                        }
                        ChromeDivider()
                        ChromeIconButton(symbol: "xmark", help: "Hide changes" + ShortcutStore.hint(for: .toggleChanges)) {
                            runtime.isDiffVisible = false
                        }
                    }
                }
            }
            .padding(.horizontal, Chrome.chromeHorizontalPadding)
            .padding(.top, Chrome.chromeTopPadding)
            .padding(.bottom, 10)

            if files.isEmpty {
                Group {
                    if isLoading {
                        // Named, so the panel says what the wait is for instead of
                        // spinning at the reader in an otherwise empty 640 by 540 pane.
                        ProgressView("Reading the diff…")
                    } else {
                        ContentUnavailableView(
                            "No changes yet",
                            systemImage: "doc.text.magnifyingglass",
                            description: Text("Files the agent changes appear here after each turn.")
                        )
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 10) {
                            ForEach(files) { file in
                                DiffFileCard(file: file, isCollapsed: collapsed.contains(file.id)) {
                                    if collapsed.contains(file.id) { collapsed.remove(file.id) } else { collapsed.insert(file.id) }
                                }
                                .equatable()
                                .id(file.id)
                            }
                        }
                        .padding(12)
                    }
                    .onChange(of: scrollRequest) {
                        guard let target = scrollTarget else { return }
                        // After layout, so the card is there to scroll to.
                        DispatchQueue.main.async {
                            withAnimation(.smooth(duration: 0.25)) { proxy.scrollTo(target, anchor: .top) }
                        }
                    }
                }
            }
        }
        .task(id: loadKey) {
            // Let the popover appear first. Kicking off git + parse + a big view build
            // in the same transaction is what made opening feel laggy: the empty
            // popover appears cheaply, content fills in right after.
            // A diff the runtime already parsed skips the wait and fills in at once.
            if !runtime.hasCachedDiff(selection: runtime.diffSelection) {
                try? await Task.sleep(for: .milliseconds(180))
                guard !Task.isCancelled else { return }
            }
            await load()
        }
    }

    private func load() async {
        let selection = runtime.diffSelection
        changedTurns = runtime.turnsWithChanges()
        isLoading = true
        defer { isLoading = false }
        let parsed = await runtime.parsedDiff(selection: selection)
        guard !Task.isCancelled else { return }
        // The tapped row's own patches carry any file the turn's diff cannot show:
        // a running turn has no end checkpoint yet, and work that was reverted or
        // moved into a worktree leaves its checkpoint diff empty.
        // Off the main thread: a tapped row's patches can run to thousands of lines,
        // and parsing them here held the popover's first frame.
        let focusEdits = runtime.diffFocusEdits
        let missing = await Task.detached(priority: .userInitiated) { focusedFiles(focusEdits, missingFrom: parsed) }.value
        guard !Task.isCancelled else { return }
        files = missing.isEmpty ? parsed : missing + parsed
        summary = FileChangeSummary(files: files)
        // The files a tapped tool row asked to see open on arrival, and the view
        // scrolls to the first of them. Everything else keeps the "first file open,
        // the rest collapsed" rule, so opening with a hundred files stays cheap.
        let focus = Set(runtime.diffFocusEdits.map(\.path))
        let focused = focus.isEmpty ? [] : files.filter { TouchedPaths.matches($0, touched: focus) }.map(\.id)
        if !focused.isEmpty {
            collapsed = Set(files.map(\.id)).subtracting(focused)
            scrollTarget = focused.first
            scrollRequest += 1
        } else if files.count > 6, collapsed.isEmpty {
            collapsed = Set(files.dropFirst().map(\.id))
        } else {
            collapsed = collapsed.intersection(files.map(\.id))
        }
    }
}

/// The patches of the edits a tapped tool row asked for, for every path the selection's
/// diff does not already cover. The row's own patch is the only copy of the change while
/// its turn runs, and it is what a turn's checkpoint diff no longer has when the agent
/// reverted the work or moved it into a worktree, so tapping a row that reports an edit
/// always opens that edit instead of "No changes yet".
private func focusedFiles(_ edits: [FileEdit], missingFrom files: [DiffFile]) -> [DiffFile] {
    var missing: [DiffFile] = []
    for edit in edits {
        guard !edit.path.isEmpty, let patch = edit.diff, !patch.isEmpty else { continue }
        let touched: Set<String> = [edit.path]
        guard !files.contains(where: { TouchedPaths.matches($0, touched: touched) }),
              !missing.contains(where: { TouchedPaths.matches($0, touched: touched) }) else { continue }
        let file = DiffParser.parseHunks(patch, path: edit.path)
        if !file.hunks.isEmpty { missing.append(file) }
    }
    return missing
}

private struct DiffFileCard: View, Equatable {
    let file: DiffFile
    let isCollapsed: Bool
    let toggle: () -> Void

    // Skip re-renders when another card's collapse state changes: the closure
    // and hunk arrays defeat SwiftUI's default diffing, so compare cheap fields.
    nonisolated static func == (lhs: DiffFileCard, rhs: DiffFileCard) -> Bool {
        lhs.file.id == rhs.file.id
            && lhs.file.additions == rhs.file.additions
            && lhs.file.deletions == rhs.file.deletions
            && lhs.file.hunks.count == rhs.file.hunks.count
            && lhs.file.change == rhs.file.change
            && lhs.file.isBinary == rhs.file.isBinary
            && lhs.isCollapsed == rhs.isCollapsed
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button(action: toggle) {
                HStack(spacing: 8) {
                    Image(systemName: "chevron.right")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(.tertiary)
                        .rotationEffect(.degrees(isCollapsed ? 0 : 90))
                    Image(systemName: symbol)
                        .foregroundStyle(color)
                    Text("\(Text(file.directory).foregroundStyle(.secondary))\(file.name)")
                        .font(.callout)
                        .lineLimit(1)
                        .truncationMode(.head)
                    Spacer(minLength: 8)
                    DiffStatLabel(additions: file.additions, deletions: file.deletions)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 9)
                .contentShape(.rect)
            }
            .buttonStyle(.plain)

            if !isCollapsed {
                if file.isBinary || file.hunks.isEmpty {
                    Text(file.isBinary ? "Binary file" : "Diff details unavailable")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 12)
                        .padding(.bottom, 10)
                } else {
                    DiffLinesView(file: file)
                        .equatable()
                        .padding(.bottom, 6)
                }
            }
        }
        .background(.quaternary.opacity(0.3), in: .rect(cornerRadius: 12, style: .continuous))
        .clipShape(.rect(cornerRadius: 12, style: .continuous))
    }

    private var symbol: String {
        switch file.change {
        case .added: "plus.circle.fill"
        case .deleted: "minus.circle.fill"
        case .renamed: "arrow.right.circle.fill"
        case .modified: "pencil.circle.fill"
        }
    }

    private var color: Color {
        switch file.change {
        case .added: .green
        case .deleted: .red
        case .renamed: .blue
        case .modified: .orange
        }
    }
}

struct DiffLinesView: View, Equatable {
    let file: DiffFile
    var showsLineNumbers = true

    // Skip re-renders when a sibling card changes: hunk arrays defeat the
    // default diff, so compare only the file identity and hunk count.
    nonisolated static func == (lhs: DiffLinesView, rhs: DiffLinesView) -> Bool {
        lhs.sectionsKey == rhs.sectionsKey && lhs.showsLineNumbers == rhs.showsLineNumbers
    }

    /// What the rows depend on, cheaply: the same path with more or fewer lines (the
    /// agent edited the file again) must rebuild them, a sibling's collapse must not.
    private var sectionsKey: String {
        "\(file.id)|\(file.hunks.count)|\(file.additions)|\(file.deletions)|\(totalLines)"
    }

    @State private var showsAll = false
    // Sections are built once per state change, not on every body pass.
    @State private var sections: [Section] = []

    /// Large files render collapsed: materializing thousands of rows at once is what
    /// makes opening a diff feel laggy. The full diff is one instant tap away.
    private static let collapsedLineLimit = 200

    private var totalLines: Int {
        file.hunks.reduce(0) { $0 + $1.lines.count }
    }

    /// One visual row group: a hunk header, a joined tinted block, or plain lines.
    /// Tinted runs merge across hunk boundaries (edits often arrive as many
    /// single-line hunks), so only the block's top and bottom lines get corners.
    /// A header that would land inside a merged run is redundant, so it is dropped.
    enum Section: Identifiable, Equatable {
        case header(String, Int)
        case tinted([DiffLine])
        case plain([DiffLine])

        var id: String {
            switch self {
            case .header(let text, let index): "h\(index)-\(text)"
            case .tinted(let lines): "t\(lines.first?.id ?? 0)"
            case .plain(let lines): "p\(lines.first?.id ?? 0)"
            }
        }
    }

    /// Builds the row groups once per state change instead of on every body pass.
    nonisolated static func makeSections(file: DiffFile, showsLineNumbers: Bool, limit: Int) -> [Section] {
        var sections: [Section] = []
        var pending: [DiffLine] = []
        var remaining = limit
        func flush() {
            if !pending.isEmpty {
                sections.append(.tinted(pending))
                pending = []
            }
        }
        for (hunkIndex, hunk) in file.hunks.enumerated() {
            guard !hunk.lines.isEmpty, remaining > 0 else { continue }
            // Only the lines the budget can show are grouped: a huge hunk behind a
            // collapsed card costs its first 200 lines, not all of them.
            let runs = lineBlocks(hunk.lines.prefix(remaining))
            // Continuing means the previous hunk ended mid-run; anything else
            // flushes and starts fresh below a new header.
            let continues = (runs.first?.isTinted == true) && !pending.isEmpty
            if !continues {
                flush()
                if showsLineNumbers {
                    sections.append(.header(hunk.header, hunkIndex))
                }
            }
            for run in runs {
                guard remaining > 0 else { break }
                if run.isTinted {
                    let take = Array(run.lines.prefix(remaining))
                    remaining -= take.count
                    pending += take
                } else {
                    flush()
                    let take = Array(run.lines.prefix(remaining))
                    remaining -= take.count
                    sections.append(.plain(take))
                }
            }
        }
        flush()
        return sections
    }

    var body: some View {
        // Lazy so a 200-line file only materializes the rows actually on screen;
        // an eager VStack here built every row during the panel-slide animation.
        LazyVStack(alignment: .leading, spacing: 0) {
            ForEach(sections) { section in
                switch section {
                case .header(let text, _):
                    Text(text)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 3)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Color.accentColor.opacity(0.06))
                case .tinted(let lines):
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(lines.enumerated()), id: \.element.id) { index, line in
                            DiffLineRow(
                                line: line,
                                showsLineNumbers: showsLineNumbers,
                                rounding: DiffLineRow.blockRounding(index: index, count: lines.count)
                            )
                        }
                    }
                case .plain(let lines):
                    ForEach(lines) { line in
                        DiffLineRow(line: line, showsLineNumbers: showsLineNumbers)
                    }
                }
            }
            if !showsAll, totalLines > Self.collapsedLineLimit {
                Button {
                    showsAll.toggle()
                } label: {
                    Text("Show all \(totalLines) lines")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 6)
                        .background(.quaternary.opacity(0.5), in: Capsule(style: .continuous))
                        .contentShape(.capsule)
                }
                .buttonStyle(.plain)
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
            }
        }
        .font(.system(size: 11.5, design: .monospaced))
        .textSelection(.enabled)
        .onChange(of: "\(sectionsKey)|\(showsAll)", initial: true) {
            sections = Self.makeSections(file: file, showsLineNumbers: showsLineNumbers, limit: showsAll ? Int.max : Self.collapsedLineLimit)
        }
    }
}

/// A maximal run of lines that share tinting, so a contiguous addition/deletion
/// block draws as one shape: corners only on its top and bottom lines.
private struct DiffLineBlock: Identifiable {
    var lines: [DiffLine]
    var isTinted: Bool

    var id: Int { lines.first?.id ?? 0 }
}

private func lineBlocks(_ lines: ArraySlice<DiffLine>) -> [DiffLineBlock] {
    var blocks: [DiffLineBlock] = []
    for line in lines {
        let tinted = line.kind == .addition || line.kind == .deletion
        if blocks.last?.isTinted == tinted {
            blocks[blocks.count - 1].lines.append(line)
        } else {
            blocks.append(DiffLineBlock(lines: [line], isTinted: tinted))
        }
    }
    return blocks
}

private struct DiffLineRow: View {
    let line: DiffLine
    let showsLineNumbers: Bool
    var rounding = UnevenRoundedRectangle(
        topLeadingRadius: 0, bottomLeadingRadius: 0,
        bottomTrailingRadius: 0, topTrailingRadius: 0, style: .continuous
    )

    var body: some View {
        // Fixed-width columns instead of one concatenated Text: numbers share
        // a right edge across all row kinds, and wrapped code stays in its own
        // column instead of flowing back under the number gutters.
        HStack(alignment: .top, spacing: 0) {
            if showsLineNumbers {
                Text(line.oldNumber.map(String.init) ?? "")
                    .frame(width: Self.numberColumnWidth, alignment: .trailing)
                    .foregroundStyle(Color.secondary.opacity(0.6))
                Text(line.newNumber.map(String.init) ?? "")
                    .frame(width: Self.numberColumnWidth, alignment: .trailing)
                    .foregroundStyle(Color.secondary.opacity(0.6))
            }
            Text(marker)
                .frame(width: Self.markerColumnWidth, alignment: .center)
                .foregroundStyle(markerColor)
            Text(line.text.isEmpty ? " " : line.text)
                .foregroundStyle(line.kind == .note ? .secondary : .primary)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.vertical, 1)
        .padding(.leading, 6)
        .padding(.trailing, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(background, in: rounding)
    }

    /// Room for a four-digit line number in the monospaced font, and for the +/- mark.
    private static let numberColumnWidth: CGFloat = 30
    private static let markerColumnWidth: CGFloat = 14

    /// Positional corners for a row inside its tinted block: only the block's
    /// top and bottom rows get corners, everything between is straight.
    static func blockRounding(index: Int, count: Int) -> UnevenRoundedRectangle {
        let radius: CGFloat = 8
        let top = index == 0
        let bottom = index == count - 1
        return UnevenRoundedRectangle(
            topLeadingRadius: top ? radius : 0,
            bottomLeadingRadius: bottom ? radius : 0,
            bottomTrailingRadius: bottom ? radius : 0,
            topTrailingRadius: top ? radius : 0,
            style: .continuous
        )
    }

    private var marker: String {
        switch line.kind {
        case .addition: "+"
        case .deletion: "−"
        case .context, .note: ""
        }
    }

    private var markerColor: Color {
        switch line.kind {
        case .addition: .green
        case .deletion: .red
        default: .secondary
        }
    }

    private var background: Color {
        switch line.kind {
        case .addition: .green.opacity(0.12)
        case .deletion: .red.opacity(0.12)
        default: .clear
        }
    }
}

/// The thread's diff as a tall, wide popover anchored to the changes tab,
/// instead of a side panel. Semitransient with a pass-through monitor: taps
/// outside close it but still reach their target, so picking another turn's
/// Review opens its diff in a single tap. Closes on Escape, the X button,
/// ⌘D, tapping the tab, or the thread going away.
@MainActor
final class DiffPopoverCoordinator: NSObject, NSPopoverDelegate {
    static let width: CGFloat = 640
    static let height: CGFloat = 540

    private let popover = NSPopover()
    private var anchor: WeakView?
    private var runtime: ThreadRuntime?
    private var desiredVisible = false
    private var monitors: [Any] = []
    /// Bumps on every show and close, so a stale didClose can never clobber a fresh open.
    private var session = 0
    private var pendingSession = 0

    override init() {
        super.init()
        popover.behavior = .semitransient
        popover.animates = true
        popover.delegate = self
    }

    /// The changes tab's own view, captured from the tab's background.
    func setAnchor(_ view: NSView) {
        anchor = WeakView(view)
        if desiredVisible { show() }
    }

    /// Drives the popover from `runtime.isDiffVisible`. Call on change and on appear.
    /// Three cases: the composer moved to another thread (close what was open and
    /// never open on arrival), first appearance (adopt the runtime without opening
    /// unless asked), or the same thread (follow the flag as before).
    func sync(isVisible: Bool, runtime: ThreadRuntime) {
        if let current = self.runtime, current !== runtime {
            close()
            if current.isDiffVisible { current.isDiffVisible = false }
            self.runtime = runtime
            seenOpenRequest = runtime.diffOpenRequest
            if runtime.isDiffVisible, !WebsiteCaptures.isEnabled { runtime.isDiffVisible = false }
            return
        }
        if self.runtime == nil {
            self.runtime = runtime
            seenOpenRequest = runtime.diffOpenRequest
            if isVisible, !WebsiteCaptures.isEnabled {
                runtime.isDiffVisible = false
                return
            }
        }
        self.runtime = runtime
        desiredVisible = isVisible
        if isVisible { show() } else { close() }
    }

    func close() {
        stopMonitors()
        desiredVisible = false
        session += 1
        pendingSession = session
        if popover.isShown { popover.performClose(nil) }
    }

    /// The composer area itself, for when the changes tab is not on screen (the
    /// follow-up queue takes its slot) and the opener brought no view of its own.
    func setFallbackAnchor(_ view: NSView) {
        fallback = WeakView(view)
        if desiredVisible { show() }
    }

    private var fallback: WeakView?
    /// The view the open popover is anchored to.
    private weak var shownOn: NSView?
    /// The runtime the open popover was shown for, the only one a close may flag.
    private weak var shownRuntime: ThreadRuntime?
    /// The thread's `diffOpenRequest` as last seen, so a counter that merely differs between two threads never opens the popover.
    private var seenOpenRequest = 0

    /// A tool row or a Review button bumped the request on the thread the
    /// coordinator already serves; a bump seen only because the composer moved
    /// to another thread is not an opener.
    func reopen(runtime: ThreadRuntime) {
        guard runtime === self.runtime else { sync(isVisible: runtime.isDiffVisible, runtime: runtime); return }
        guard runtime.diffOpenRequest != seenOpenRequest else { return }
        seenOpenRequest = runtime.diffOpenRequest
        desiredVisible = true
        if popover.isShown {
            if shownOn === resolvedAnchor()?.view { return }
            stopMonitors()
            session += 1
            pendingSession = session
            popover.close()
        }
        show()
    }

    /// A tool row or a Review button opens the popover on itself, on the side it asked
    /// for (below a row, above a button); the tab and ⌘D open it above the tab, or
    /// above the composer while the queue has the tab's slot.
    private func resolvedAnchor() -> (view: NSView, edge: NSRectEdge)? {
        if let runtime, let view = runtime.diffAnchor?.value, view.window != nil {
            return (view, runtime.diffAnchorEdge)
        }
        guard let view = [anchor?.value, fallback?.value].compactMap({ $0 }).first(where: { $0.window != nil }) else {
            return nil
        }
        return (view, .maxY)
    }

    private func show() {
        guard let runtime, let target = resolvedAnchor() else { return }
        session += 1
        guard !popover.isShown else { return }
        popover.setFixedContent(DiffInspector(runtime: runtime), size: NSSize(width: Self.width, height: Self.height))
        startMonitors()
        shownOn = target.view
        shownRuntime = runtime
        popover.show(relativeTo: target.view.bounds, of: target.view, preferredEdge: target.edge)
    }

    // MARK: - Dismissal

    nonisolated func popoverDidClose(_ notification: Notification) {
        Task { @MainActor [weak self] in
            guard let self else { return }
            self.stopMonitors()
            self.desiredVisible = false
            // Only the close that posted this may clear the flag and drop the content:
            // a Review tap that already reopened (newer session) must survive. Dropping
            // the content releases the inspector's view tree, which otherwise stays
            // alive, observing the runtime, until the next open replaces it.
            if self.session == self.pendingSession {
                if let shown = self.shownRuntime, shown.isDiffVisible { shown.isDiffVisible = false }
                // The content goes with the close: a hosting view left in the popover kept
                // its inspector alive, observing the runtime and re-rendering on every
                // change while nothing was on screen.
                self.popover.contentViewController = nil
            }
            self.shownRuntime = nil
        }
    }

    /// Clicks inside the panel pass through; any other click dismisses first
    /// but still reaches its target (e.g. another turn's Review button).
    private func handleMouseDown(_ event: NSEvent) -> NSEvent? {
        if event.window === popover.contentViewController?.view.window { return event }
        close()
        return event
    }

    private func handleKeyDown(_ event: NSEvent) -> NSEvent? {
        guard event.keyCode == 53 else { return event } // Escape
        close()
        return nil
    }

    private func startMonitors() {
        guard monitors.isEmpty else { return }
        if let monitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown], handler: { [weak self] event in
            self?.handleMouseDown(event) ?? event
        }) {
            monitors.append(monitor)
        }
        if let monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown, handler: { [weak self] event in
            self?.handleKeyDown(event) ?? event
        }) {
            monitors.append(monitor)
        }
    }

    private func stopMonitors() {
        for monitor in monitors { NSEvent.removeMonitor(monitor) }
        monitors.removeAll()
    }
}
