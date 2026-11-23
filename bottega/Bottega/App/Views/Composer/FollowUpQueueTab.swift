// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// The queued steering prompts as a tab rising from the top of the chat box. The composer draws
/// over its lower edge, so it reads as part of the box. Each row is one stacked follow-up: it
/// is sent as a direct user chat message once the running turn finishes, in order from top to
/// bottom.
struct FollowUpQueueTab: View {
    /// How far the composer draws over the tab's lower edge: the stack that holds the two
    /// pulls the box up by this much (see `ComposerArea`), and the tab pads its bottom by as
    /// much again so its own content stays clear of the box.
    static let overlap: CGFloat = 14

    /// The rows the tab shows before the list scrolls inside it: past that the tab would
    /// cover the chat, and the prompts the reader is waiting on are the ones at the top.
    static let maxRows = 7

    /// The height the open list is cut to: the top of its content - the rule, the gap and
    /// the tab's own padding - plus `maxRows` rows of the size the rows actually are.
    /// Taken from `rowHeights`, so the cut lands on a whole row whatever a row's height
    /// is. Below the cap, or before anything has been measured, the natural `listHeight`
    /// stands as it is.
    static func cappedHeight(_ listHeight: CGFloat, rowHeights: [UUID: CGFloat], count: Int) -> CGFloat {
        guard count > maxRows, !rowHeights.isEmpty else { return listHeight }
        let rows = rowHeights.values.reduce(0, +)
        let top = max(0, listHeight - rows)
        return top + CGFloat(maxRows) * rows / CGFloat(rowHeights.count)
    }

    let runtime: ThreadRuntime

    /// Whether the queued rows are folded away under the title: kept on the thread, so
    /// the fold survives the tab leaving the screen whenever the queue empties.
    private var isCollapsed: Bool { runtime.followUpsCollapsed }

    /// The live reorder: the grabbed prompt, the pointer's travel since the
    /// grab, and how far its slot has already moved to meet it (see `RowDrag`).
    @State private var drag = RowDrag<UUID>()
    /// Watches for the mouse going up while a row is held: a gesture cancelled from under
    /// the pointer reports no end, and the row would stay lifted.
    @State private var mouseUpMonitor: Any?
    /// Each row's height including its padding: one row's slot in the stack.
    @State private var rowHeights: [UUID: CGFloat] = [:]
    /// The row the held row would pair with while nudged right, lit up as the target.
    @State private var pairTarget: UUID?
    /// The rows' natural height, so the fold can animate to and from exactly it.
    @State private var listHeight: CGFloat = 0

    var body: some View {
        let shape = UnevenRoundedRectangle(topLeadingRadius: 12, topTrailingRadius: 12, style: .continuous)
        VStack(alignment: .leading, spacing: 0) {
            // The whole title line folds the queue and opens it again; the chevron at
            // its end only says which way it will go: down to close while open, up to
            // reopen while collapsed.
            Button {
                withAnimation(Chrome.panelSlide) { runtime.followUpsCollapsed.toggle() }
            } label: {
                HStack(spacing: 7) {
                    Image(systemName: "arrow.turn.down.right")
                        .chromeInlineIconFont()
                        .foregroundStyle(Chrome.secondaryText)
                    Text(verbatim: runtime.followUps.count == 1 ? "1 follow-up" : "\(runtime.followUps.count) follow-ups")
                        .foregroundStyle(Chrome.primaryText.opacity(0.9))
                    Text(verbatim: "queued")
                        .foregroundStyle(Chrome.secondaryText)
                    Spacer(minLength: 8)
                    Image(systemName: "chevron.down")
                        .chromeInlineIconFont()
                        .foregroundStyle(Chrome.secondaryText)
                        .frame(width: 22, height: 22)
                        .rotationEffect(.degrees(isCollapsed ? 180 : 0))
                }
                .zoomedFont(size: 12, weight: .medium, monospacedDigit: true)
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            // Kept out of the key view loop on purpose. SwiftUI's default key-view-loop builder
            // walks focusable views in reading order, and with this line sitting flush above
            // the row's trash button at the same x it never advanced past it: the main
            // thread spun in that rebuild forever (the Sept 15 2026 freezes). None of the tab's
            // controls are meant to be tabbed to, so none of them join the loop.
            .focusable(false)
            .help(isCollapsed ? "Expand queued follow-ups" : "Collapse queued follow-ups")
            .accessibilityLabel(Text(verbatim: runtime.followUps.count == 1 ? "1 queued follow-up" : "\(runtime.followUps.count) queued follow-ups"))
            .accessibilityValue(Text(isCollapsed ? "Collapsed" : "Expanded"))

            // The rows stay in place and fold: a clip animates between zero
            // and their measured height while they fade, so nothing is ever
            // removed mid-animation to linger over the composer as a ghost.
            ScrollView(.vertical) {
                VStack(alignment: .leading, spacing: 4) {
                    Divider().opacity(0.5)

                    // Rows carry their own vertical padding and rule, so a row's
                    // measured height is exactly its slot and the reorder maths
                    // never has to know about stack spacing.
                    VStack(alignment: .leading, spacing: 0) {
                        let numbers = Self.numbers(for: runtime.followUps)
                        ForEach(Array(runtime.followUps.enumerated()), id: \.element.id) { index, prompt in
                            let isDragged = drag.id == prompt.id
                            let previous = index > 0 ? runtime.followUps[index - 1] : nil
                            let next = index + 1 < runtime.followUps.count ? runtime.followUps[index + 1] : nil
                            let isBundledWithPrevious = prompt.bundleID != nil && prompt.bundleID == previous?.bundleID
                            let bundledWithNext = prompt.bundleID != nil && prompt.bundleID == next?.bundleID
                            FollowUpRow(
                                position: numbers[prompt.id] ?? index + 1,
                                prompt: prompt,
                                runtime: runtime,
                                isDragged: isDragged,
                                showsRule: prompt.id != runtime.followUps.last?.id && !isDragged && !bundledWithNext,
                                isBundledWithPrevious: isBundledWithPrevious,
                                isBundledWithNext: bundledWithNext,
                                isPairTarget: pairTarget == prompt.id,
                                isPairing: isDragged && pairTarget != nil,
                                onDragChanged: { translation in dragChanged(prompt.id, translation: translation) },
                                onDragEnded: { dragEnded() }
                            )
                            .modifier(RowHeightReporter(id: prompt.id, heights: $rowHeights))
                            // Only while dragging: settling is the only reader. In pairing
                            // mode the held row indents to show it will pair on release.
                            .offset(x: pairTarget != nil && isDragged ? 14 : 0, y: isDragged ? drag.visualOffset : 0)
                            .animation(Self.slide, value: pairTarget != nil)
                            .zIndex(isDragged ? 1 : 0)
                        }
                    }
                }
                .padding(.top, 6)
                // Measured inside the scroll view, where the height is the rows' own: the
                // frame below proposes the capped height to the scroll view, and a scroll
                // view answers that proposal rather than its content.
                .onGeometryChange(for: CGFloat.self, of: { $0.size.height }) { height in
                    if height > 0 { listHeight = height }
                }
            }
            .scrollBounceBehavior(.basedOnSize)
            // The cap: past seven rows the list scrolls inside the tab instead of covering
            // the chat. The fold animates to the capped height, which is what the reader
            // sees; `listHeight` itself stays the rows' natural height.
            .frame(height: isCollapsed ? 0 : Self.cappedHeight(listHeight, rowHeights: rowHeights, count: runtime.followUps.count), alignment: .top)
            .opacity(isCollapsed ? 0 : 1)
            // The fold needs the clip, but the rows inside it must not be shaved: the
            // lifted row's shadow hangs below it, and the pair target's scale reaches
            // past the row at the sides. The mask runs a little past the tab's own
            // edges while the list is open, closing in with the fold.
            .mask {
                Rectangle()
                    .padding(.horizontal, -32)
                    .padding(.vertical, isCollapsed ? 0 : -16)
            }
            .allowsHitTesting(!isCollapsed)
            .accessibilityHidden(isCollapsed)
        }
        .padding(.horizontal, 12)
        .padding(.top, 7)
        .padding(.bottom, 7 + Self.overlap)
        .frame(maxWidth: 560)
        .glassEffect(.regular, in: shape)
        .contentShape(shape)
        .onChange(of: runtime.followUps.map(\.id)) { _, ids in
            // A row that left mid-drag (deleted, or sent) ends the drag cleanly, and its
            // measured height goes with it.
            if let id = drag.id, !ids.contains(id) { dragEnded() }
            if rowHeights.count > ids.count {
                let kept = Set(ids)
                rowHeights = rowHeights.filter { kept.contains($0.key) }
            }
        }
        .onChange(of: drag.id) { _, id in
            if id != nil {
                guard mouseUpMonitor == nil else { return }
                mouseUpMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseUp]) { event in
                    // After the gesture's own end, which settles the row; this only
                    // catches the drags it never reports.
                    DispatchQueue.main.async { dragEnded() }
                    return event
                }
            } else if let monitor = mouseUpMonitor {
                NSEvent.removeMonitor(monitor)
                mouseUpMonitor = nil
            }
        }
        .onDisappear {
            if let monitor = mouseUpMonitor {
                NSEvent.removeMonitor(monitor)
                mouseUpMonitor = nil
            }
        }
    }

    /// Reports its row's height for the reorder maths. Always on: switching the modifier
    /// in as a drag began changed every row's structure under the grip mid-gesture, and a
    /// gesture torn down that way never ends, leaving the row lifted for good.
    private struct RowHeightReporter: ViewModifier {
        let id: UUID
        @Binding var heights: [UUID: CGFloat]

        func body(content: Content) -> some View {
            content.onGeometryChange(for: CGFloat.self, of: { $0.size.height }) { height in
                if heights[id] != height { heights[id] = height }
            }
        }
    }

    // MARK: - Reorder

    /// Neighbours sliding out of the grabbed row's way.
    private static let slide = Animation.spring(response: 0.28, dampingFraction: 0.82)

    /// The middle share of a neighbour's slot over which the held row pairs with it rather
    /// than swapping past it: a quarter either side of its centre.
    private static let pairBand: CGFloat = 0.25

    /// One shared number per bundle: the number climbs when a prompt starts a new
    /// bundle or has none at all.
    private static func numbers(for prompts: [FollowUpPrompt]) -> [UUID: Int] {
        var numbers: [UUID: Int] = [:]
        var number = 0
        var previous: UUID?
        var hadPrevious = false
        for prompt in prompts {
            if prompt.bundleID == nil || !hadPrevious || prompt.bundleID != previous {
                number += 1
            }
            numbers[prompt.id] = number
            previous = prompt.bundleID
            hadPrevious = true
        }
        return numbers
    }

    /// The pointer has moved `translation` since the grab. The grabbed row follows it exactly; over the middle of a neighbour it pairs with that row, past the neighbour's far quarter the two swap. Sideways travel plays no part.
    private func dragChanged(_ id: UUID, translation: CGSize) {
        if drag.id != id {
            drag = RowDrag(id: id)
            pairTarget = nil
            NSCursor.closedHand.push()
        }
        var transaction = Transaction()
        transaction.disablesAnimations = true
        withTransaction(transaction) { drag.translation = translation.height }

        // Past the far quarter of a neighbour the rows swap; over its middle the two pair.
        let moved = drag.settle(order: runtime.followUps.map(\.id), heights: rowHeights, fallbackHeight: 32, pastCentre: Self.pairBand) { neighbour, placeAfter in
            // The neighbour slides and the grabbed row's slot moves in the same animation
            // as its compensation, so it stays put under the pointer while the list flows.
            withAnimation(Self.slide) {
                runtime.moveFollowUp(id, to: neighbour, placeAfter: placeAfter)
            }
        }
        if moved {
            NSHapticFeedbackManager.defaultPerformer.perform(.alignment, performanceTime: .now)
        }

        // Pulling a linked row more than half its height out of its slot unlinks it, so a
        // pair with no other rows to swap past can still be broken by drag; hovering the
        // neighbour's middle again pairs them back on release.
        if let held = runtime.followUps.first(where: { $0.id == id }), held.bundleID != nil,
            abs(drag.visualOffset) > (rowHeights[id] ?? 32) * 0.6
        {
            withAnimation(Self.slide) { runtime.unbundleFollowUp(id) }
            NSHapticFeedbackManager.defaultPerformer.perform(.levelChange, performanceTime: .now)
        }

        // Pairing: the held row's centre sits within the middle half of the neighbour it is
        // heading for, wherever the pointer is sideways.
        let ids = runtime.followUps.map(\.id)
        var target: UUID?
        if let index = ids.firstIndex(of: id) {
            let own = rowHeights[id] ?? 32
            let offset = drag.visualOffset
            var neighbour: UUID?
            if offset > 0, index + 1 < ids.count { neighbour = ids[index + 1] }
            if offset < 0, index > 0 { neighbour = ids[index - 1] }
            if let neighbour {
                let slot = rowHeights[neighbour] ?? 32
                let toCentre = (own + slot) / 2
                if abs(abs(offset) - toCentre) <= slot * Self.pairBand { target = neighbour }
            }
        }
        if target != pairTarget {
            if target != nil, pairTarget == nil {
                NSHapticFeedbackManager.defaultPerformer.perform(.alignment, performanceTime: .now)
            }
            withAnimation(Self.slide) { pairTarget = target }
        }
    }

    private func dragEnded() {
        if let held = drag.id, let target = pairTarget {
            withAnimation(.spring(response: 0.36, dampingFraction: 0.78)) {
                runtime.bundleFollowUp(held, onto: target)
            }
            NSHapticFeedbackManager.defaultPerformer.perform(.levelChange, performanceTime: .now)
            pairTarget = nil
        }
        guard drag.id != nil else { return }
        NSCursor.pop()
        // The offset animates from wherever the pointer let go to the row's
        // slot, so the row settles instead of snapping.
        withAnimation(.spring(response: 0.32, dampingFraction: 0.8)) { drag = RowDrag() }
    }
}

private struct FollowUpRow: View {
    let position: Int
    let prompt: FollowUpPrompt
    let runtime: ThreadRuntime
    /// Lifted and following the pointer.
    let isDragged: Bool
    /// The rule under the row, off for the last row and while lifted.
    let showsRule: Bool
    /// The previous row shares this row's bundle: show the link mark, a tap on it unlinks.
    let isBundledWithPrevious: Bool
    /// The next row shares this row's bundle: the number and grip turn blue as the head of the pair.
    let isBundledWithNext: Bool
    /// The held row hovers over this row while pairing: light it up.
    let isPairTarget: Bool
    /// This row is the one held, and it hovers over a row it will pair with.
    let isPairing: Bool
    let onDragChanged: (CGSize) -> Void
    let onDragEnded: () -> Void

    /// One preview panel for this row's thumbnails, so every photo opens.
    @State private var preview = AttachmentPreviewSlot()
    /// The editor popover for this row, anchored to its pencil button.
    @State private var editor = PromptEditorPopover<FollowUpEditor>()

    /// Whether the pointer is over the reorder grip, for the grab cursor.
    @State private var isHoveringGrip = false
    /// Held while the grip's drag runs; SwiftUI resets it when the gesture ends or is
    /// cancelled, and a cancelled gesture calls no `onEnded` of its own.
    @GestureState private var isGrabbing = false
    /// The window's text size: the prose's lift grows with its font.
    @Environment(\.chatZoom) private var zoom

    /// How far the prose is lifted to centre its x-height on the row's line: half the
    /// gap between cap height and x-height of its 12 pt font, on the half point, at the
    /// stock text size.
    nonisolated private static let proseLift: CGFloat = {
        let font = NSFont.systemFont(ofSize: 12)
        return ((font.capHeight - font.xHeight) / 2 * 2).rounded() / 2
    }()

    var body: some View {
        // Which follow-up is open for editing is the thread's, so the editor survives the
        // row: leaving the thread closes the popover only, and the row coming back reopens
        // it on the edit as it was left.
        let isEditing = runtime.followUpEdit?.id == prompt.id
        // The lift at the window's text size, as a value: the guide below is worked out
        // off the main actor.
        let proseLift = ChatZoom.scaled(Self.proseLift, by: zoom)
        // One shared center line: the number, grip, thumbnails, text and
        // buttons all center on it, so single-line rows read as one line.
        HStack(alignment: .center, spacing: 8) {
            // A joined row, a pairing target and the held row while pairing all show the
            // link instead of a number: the bundle goes as one prompt under one number.
            Group {
                if isBundledWithPrevious || isPairTarget || isPairing {
                    if isBundledWithPrevious {
                        Button {
                            withAnimation(.spring(response: 0.3, dampingFraction: 0.8)) {
                                runtime.unbundleFollowUp(prompt.id)
                            }
                            NSHapticFeedbackManager.defaultPerformer.perform(.levelChange, performanceTime: .now)
                        } label: {
                            Image(systemName: "link")
                                .zoomedFont(size: 10, weight: .semibold)
                                .foregroundStyle(Chrome.accent)
                        }
                        .buttonStyle(.plain)
                        .focusable(false)
                        .contentShape(.rect)
                        .help("Unlink from the follow-up above")
                        .accessibilityLabel(Text("Unlink"))
                    } else {
                        Image(systemName: "link")
                            .zoomedFont(size: 10, weight: .semibold)
                            .foregroundStyle(Chrome.accent)
                            .accessibilityHidden(true)
                    }
                } else {
                    Text(verbatim: "\(position)")
                        .zoomedFont(size: 11, weight: .medium, monospacedDigit: true)
                        .foregroundStyle(isBundledWithNext ? Chrome.accent : Chrome.secondaryText)
                        .accessibilityHidden(true)
                }
            }
            // Wide enough for two digits at the window's text size.
            .frame(width: ChatZoom.scaled(14, by: zoom), alignment: .trailing)
            Image(systemName: "line.3.horizontal")
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(isBundledWithNext ? Chrome.accent.opacity(isHoveringGrip ? 1 : 0.85) : Chrome.secondaryText.opacity(isHoveringGrip ? 1 : 0.7))
                .frame(width: 28, height: 22)
                .contentShape(.rect)
                .onHover { hovering in
                    isHoveringGrip = hovering
                    // The grab cursor is the drag's while a drag is on.
                    guard !isDragged else { return }
                    if hovering { NSCursor.openHand.push() } else { NSCursor.pop() }
                }
                .onDisappear {
                    if isHoveringGrip { NSCursor.pop() }
                    isHoveringGrip = false
                }
                .onChange(of: isDragged) { _, dragging in
                    // Released away from the grip: the hover's open hand is
                    // still pushed with no leave to pop it.
                    if !dragging, !isHoveringGrip { NSCursor.pop() }
                }
                .gesture(
                    DragGesture(minimumDistance: 1, coordinateSpace: .global)
                        .updating($isGrabbing) { _, grabbing, _ in grabbing = true }
                        .onChanged { value in onDragChanged(value.translation) }
                        .onEnded { _ in onDragEnded() }
                )
                .onChange(of: isGrabbing) { _, grabbing in
                    if !grabbing, isDragged { onDragEnded() }
                }
                .help("Drag above or below to reorder · drop onto the middle of another to send them together")
                .accessibilityLabel(Text("Drag to reorder"))
            if !prompt.attachments.isEmpty {
                // No anchor view under the thumbnails or the strip: nothing in this tab
                // may join SwiftUI's key-view-loop walk (see `WindowRectAnchor`). The
                // panel hangs from the strip's rect on the window instead.
                HStack(spacing: 4) {
                    ForEach(prompt.attachments) { attachment in
                        AttachmentThumbnail(attachment: attachment, size: 22, preview: preview, anchorless: true)
                    }
                }
                .windowRectAnchor { preview.setAnchor(windowRect: $0) }
                .onDisappear { preview.close() }
            }
            VStack(alignment: .leading, spacing: 4) {
                if prompt.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Text(verbatim: attachmentOnlyLabel)
                        .zoomedFont(size: 12)
                        .foregroundStyle(Chrome.secondaryText)
                        .lineLimit(2)
                        .truncationMode(.tail)
                } else {
                    // Not selectable on purpose: selection mounts an AppKit text view,
                    // and this tab keeps every AppKit view out of the key-view-loop walk
                    // (see `WindowRectAnchor`). The pencil opens the full text to copy.
                    Text(verbatim: prompt.text)
                        .zoomedFont(size: 12)
                        .foregroundStyle(Chrome.primaryText.opacity(0.9))
                        .lineLimit(2)
                        .truncationMode(.tail)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            // The line box centres on cap height, which is right for the digit and the
            // symbols, but prose is read by its x-height and sat a hair low beside them.
            .alignmentGuide(VerticalAlignment.center) { $0[VerticalAlignment.center] + proseLift }
            Spacer(minLength: 4)
            HStack(spacing: 0) {
                // Native hands run inside the lead's turn, so that turn is not stopped for
                // a queued prompt; it goes the moment they have reported.
                let sendHelp = runtime.hasWorkingNativeHands && runtime.isRunning
                    ? "Send as soon as the hands report"
                    : runtime.isRunning ? "Send now (stops the running turn)" : "Send now"
                QueueIconButton(symbol: "paperplane", help: sendHelp) {
                    runtime.sendFollowUpNow(prompt.id)
                }
                QueueIconButton(symbol: "pencil", help: "Edit follow-up") {
                    // The pencil toggles: a second tap while open closes the editor.
                    runtime.followUpEdit = isEditing ? nil : PromptEdit(id: prompt.id, text: prompt.text, attachments: prompt.attachments)
                }
                .windowRectAnchor { rect in
                    editor.setAnchor(windowRect: rect)
                    // The rect lands after the row appears: the moment a reopened
                    // thread can show the editor again.
                    syncEditor(isEditing)
                }
                QueueIconButton(symbol: "trash", help: "Delete follow-up") {
                    runtime.removeFollowUp(prompt.id)
                }
            }
            .fixedSize()
        }
        // Tight rows: a one-line follow-up is 32 tall, the grip and buttons 22.
        .padding(.vertical, 5)
        .overlay(alignment: .bottom) {
            if showsRule { Divider().opacity(0.35) }
        }
        // Lifted: a touch larger with a shadow, over an opaque glass so the
        // rows sliding underneath never show through. A capsule, like the user's
        // own bubbles in the chat above.
        .background {
            if isPairTarget {
                // Lit and framed: the held row will join this one on release.
                Capsule(style: .continuous)
                    .fill(Chrome.accent.opacity(0.18))
                    .overlay {
                        Capsule(style: .continuous)
                            .strokeBorder(Chrome.accent.opacity(0.65), lineWidth: 1.5)
                    }
            }
            if isDragged {
                Capsule(style: .continuous)
                    .fill(Chrome.overlay(0.12))
                    .shadow(color: .black.opacity(0.28), radius: 10, y: 4)
            }
        }
        // A scale above the scroll view's edge would be cut off there, so the lift reads
        // off the shadow and the deep offset instead.
        .scaleEffect(isPairTarget ? 1.008 : isDragged ? 1.01 : 1)
        .animation(.spring(response: 0.25, dampingFraction: 0.8), value: isDragged)
        .animation(.spring(response: 0.25, dampingFraction: 0.8), value: isPairTarget)
        .onChange(of: prompt.attachments) {
            preview.retire(except: Set(prompt.attachments.map(\.id)))
        }
        .onChange(of: isEditing) { _, editing in syncEditor(editing) }
        .onDisappear {
            preview.close()
            editor.close()
        }
    }

    private func syncEditor(_ isEditing: Bool) {
        editor.sync(isEditing ? runtime.followUpEdit : nil, dismiss: { runtime.followUpEdit = nil }) { edit in
            FollowUpEditor(edit: edit, runtime: runtime)
        }
    }

    private var attachmentOnlyLabel: String {
        let images = prompt.attachments.filter(\.isImage).count
        let files = prompt.attachments.count - images
        var parts: [String] = []
        if images == 1 { parts.append("1 image") } else if images > 1 { parts.append("\(images) images") }
        if files == 1 { parts.append("1 file") } else if files > 1 { parts.append("\(files) files") }
        guard !parts.isEmpty else { return "Empty follow-up" }
        return parts.joined(separator: ", ")
    }
}

private struct QueueIconButton: View {
    let symbol: String
    let help: String
    var isEnabled = true
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(Chrome.secondaryText.opacity(isEnabled ? 1 : 0.35))
                .frame(width: 22, height: 22)
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .focusable(false)
        .disabled(!isEnabled)
        .help(help)
        .accessibilityLabel(Text(help))
    }
}
