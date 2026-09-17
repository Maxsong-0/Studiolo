import AppKit
import SwiftUI

/// Every key combination Droppy Code answers to, laid out like Droppy's Shortcuts page:
/// a section header with its tile, one card of rows, and one recorder per row.
struct ShortcutsSettingsPage: View {
    var body: some View {
        LazyVStack(alignment: .leading, spacing: Chrome.sectionSpacing) {
            ForEach(AppShortcut.Section.allCases) { section in
                ShortcutSectionCard(section: section)
            }
            FixedShortcutsCard()
        }
        .focusEffectDisabled()
    }
}

private struct ShortcutSectionHeader<Accessory: View>: View {
    let title: String
    let symbol: String
    @ViewBuilder var accessory: Accessory

    var body: some View {
        HStack(alignment: .center, spacing: 8) {
            SidebarIconBadge {
                SidebarSymbol(symbol)
            }
            Text(verbatim: title)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Spacer(minLength: 8)
            accessory
        }
        .frame(height: 22)
    }
}

private struct ShortcutSectionCard: View {
    let section: AppShortcut.Section

    var body: some View {
        let store = ShortcutStore.shared
        VStack(alignment: .leading, spacing: Chrome.sectionHeaderSpacing) {
            ShortcutSectionHeader(title: section.title, symbol: section.symbol) {
                if store.hasCustomizations(in: section) {
                    Button("Reset") {
                        withAnimation(Chrome.hover) { store.reset(section) }
                    }
                    .buttonStyle(ShortcutPillButtonStyle())
                    .transition(.opacity)
                }
            }
            ChromeCard {
                ForEach(Array(section.shortcuts.enumerated()), id: \.element) { offset, shortcut in
                    if offset > 0 {
                        Divider().padding(.leading, ShortcutRowMetrics.dividerLeadingInset)
                    }
                    ShortcutEntryRow(shortcut: shortcut)
                }
            }
        }
    }
}

/// Keys the composer handles itself. They read like the rows above but cannot be changed.
private struct FixedShortcutsCard: View {
    private let rows: [(title: String, detail: String, chord: String)] = [
        ("Send", "Sends the message in the composer.", "Return"),
        ("New line", "Adds a line without sending.", "⇧ Return"),
        ("Steer a chat", "While a turn runs, stops it and sends right away.", "Return"),
        ("Recall an earlier prompt", "Works in an empty composer.", "↑"),
        ("Approve a request", "Allows what the agent asked for.", "⌘ Return"),
        ("Jump to a thread", "Selects one of the first nine threads.", "⌘ 1–9"),
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: Chrome.sectionHeaderSpacing) {
            ShortcutSectionHeader(title: "Composer", symbol: "text.cursor") {
                EmptyView()
            }
            ChromeCard {
                ForEach(Array(rows.enumerated()), id: \.offset) { offset, row in
                    if offset > 0 {
                        Divider().padding(.leading, ShortcutRowMetrics.dividerLeadingInset)
                    }
                    ShortcutSettingsRow(title: row.title, detail: row.detail) {
                        Text(verbatim: row.chord)
                            .font(.system(size: 13, weight: .semibold))
                            .foregroundStyle(Chrome.secondaryText)
                            .frame(width: ShortcutRowMetrics.controlWidth, height: ShortcutRowMetrics.controlHeight)
                            .background(
                                RoundedRectangle(cornerRadius: 8, style: .continuous)
                                    .fill(Chrome.overlay(0.07))
                            )
                            .help("Built in")
                    }
                }
            }
        }
    }
}

enum ShortcutRowMetrics {
    static let rowPadding: CGFloat = 12
    static let dividerLeadingInset: CGFloat = 16
    static let controlSpacing: CGFloat = 16
    static let controlWidth: CGFloat = 168
    static let controlHeight: CGFloat = 24
}

/// The name on the left, one line, and the control right-aligned on the same line.
private struct ShortcutSettingsRow<Accessory: View>: View {
    let title: String
    let detail: String
    @ViewBuilder let accessory: Accessory

    var body: some View {
        HStack(alignment: .center, spacing: ShortcutRowMetrics.controlSpacing) {
            Text(verbatim: title)
                .font(.callout)
                .foregroundStyle(.primary)
                .lineLimit(1)
                .truncationMode(.tail)
                .help(detail)
                .layoutPriority(0)
            Spacer(minLength: 0)
            accessory
                .layoutPriority(1)
                .fixedSize(horizontal: true, vertical: false)
        }
        .frame(minHeight: 24)
        .padding(ShortcutRowMetrics.rowPadding)
    }
}

private struct ShortcutEntryRow: View {
    let shortcut: AppShortcut

    var body: some View {
        ShortcutSettingsRow(title: shortcut.title, detail: shortcut.detail) {
            ShortcutRecorder(shortcut: shortcut)
        }
    }
}

// MARK: - Recorder

/// One fixed-width button: the chord when there is one, "Record shortcut" when there is not.
/// Clicking it opens the native popover with the keycaps, and Clear and Reset to default under them.
private struct ShortcutRecorder: View {
    let shortcut: AppShortcut

    @State private var session = ShortcutRecordingSession()
    @State private var isPresenting = false

    var body: some View {
        let store = ShortcutStore.shared
        let chord = store.chord(for: shortcut)
        Button {
            isPresenting.toggle()
        } label: {
            Group {
                if let chord {
                    HStack(spacing: 7) {
                        ForEach(Keycap.caps(for: chord)) { cap in
                            Text(verbatim: cap.symbol)
                                .font(.system(size: 13, weight: .semibold))
                                .lineLimit(1)
                                .fixedSize()
                        }
                    }
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(Text(verbatim: chord.description))
                } else {
                    Text("Record shortcut")
                        .font(.system(size: 12, weight: .medium))
                        .lineLimit(1)
                }
            }
            .frame(width: ShortcutRowMetrics.controlWidth, height: ShortcutRowMetrics.controlHeight)
            .contentShape(Rectangle())
        }
        .buttonStyle(ShortcutCombinedControlStyle(isSet: chord != nil, isRecording: isPresenting))
        .animation(.spring(response: 0.3, dampingFraction: 0.8), value: chord)
        .popover(isPresented: $isPresenting, arrowEdge: .top) {
            ShortcutRecordingPopoverContent(session: session)
                .presentedChrome()
        }
        .onChange(of: isPresenting) { _, presenting in
            if presenting {
                session.begin(shortcut: shortcut, actions: actions(current: chord)) {
                    isPresenting = false
                } onCommit: { captured in
                    store.set(captured, for: shortcut)
                }
            } else {
                session.cancel()
            }
        }
        .onDisappear { session.cancel() }
    }

    private func actions(current: KeyChord?) -> [ShortcutRecordingAction] {
        let store = ShortcutStore.shared
        var actions: [ShortcutRecordingAction] = []
        if current != nil {
            actions.append(ShortcutRecordingAction(id: "clear", title: "Clear", isDestructive: true) {
                Haptics.perform(.generic)
                store.set(nil, for: shortcut)
                isPresenting = false
            })
        }
        if store.isCustomized(shortcut), shortcut.defaultChord != nil {
            actions.append(ShortcutRecordingAction(id: "reset", title: "Reset to default", isDestructive: false) {
                Haptics.perform(.generic)
                store.reset(shortcut)
                isPresenting = false
            })
        }
        return actions
    }
}

/// Flat washes rather than glass: one control per row, many rows at once.
private struct ShortcutCombinedControlStyle: ButtonStyle {
    let isSet: Bool
    let isRecording: Bool

    func makeBody(configuration: Configuration) -> some View {
        ShortcutCombinedControlContent(configuration: configuration, isSet: isSet, isRecording: isRecording)
    }
}

private struct ShortcutCombinedControlContent: View {
    let configuration: ButtonStyleConfiguration
    let isSet: Bool
    let isRecording: Bool

    @State private var isHovering = false
    @Environment(\.isEnabled) private var isEnabled

    private var fill: Color {
        if isRecording { return Chrome.accent.opacity(configuration.isPressed ? 0.34 : 0.26) }
        if isSet { return Chrome.overlay(configuration.isPressed ? 0.34 : (isHovering ? 0.30 : 0.24)) }
        return Chrome.overlay(configuration.isPressed ? 0.14 : (isHovering ? 0.11 : 0.07))
    }

    private var ink: Color {
        guard isEnabled else { return Chrome.secondaryText.opacity(0.6) }
        return isSet || isRecording ? Chrome.primaryText : Chrome.secondaryText
    }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 8, style: .continuous)
        configuration.label
            .foregroundStyle(ink)
            .background(shape.fill(fill))
            .contentShape(shape)
            .scaleEffect(configuration.isPressed ? 0.97 : 1)
            .animation(.easeOut(duration: 0.08), value: configuration.isPressed)
            .animation(Chrome.hover, value: isHovering)
            .animation(Chrome.hover, value: isRecording)
            .onHover { isHovering = $0 }
    }
}

/// The small pill for a section's Reset.
private struct ShortcutPillButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(Chrome.primaryText)
            .padding(.horizontal, 10)
            .frame(height: 22)
            .background(Capsule(style: .continuous).fill(Chrome.overlay(configuration.isPressed ? 0.16 : 0.1)))
            .contentShape(Capsule(style: .continuous))
    }
}

// MARK: - Keycaps

struct Keycap: Identifiable, Equatable {
    enum Phase: Equatable {
        case pressed
        case ghost
    }

    let id: String
    let symbol: String
    let name: String?
    let phase: Phase

    var isModifier: Bool { name != nil }

    static func modifierCaps(for flags: NSEvent.ModifierFlags, phase: Phase) -> [Keycap] {
        var caps: [Keycap] = []
        if flags.contains(.control) { caps.append(Keycap(id: "control", symbol: "⌃", name: "ctrl", phase: phase)) }
        if flags.contains(.option) { caps.append(Keycap(id: "option", symbol: "⌥", name: "opt", phase: phase)) }
        if flags.contains(.shift) { caps.append(Keycap(id: "shift", symbol: "⇧", name: "shift", phase: phase)) }
        if flags.contains(.command) { caps.append(Keycap(id: "command", symbol: "⌘", name: "cmd", phase: phase)) }
        return caps
    }

    static func keyCap(keyCode: Int, phase: Phase) -> Keycap {
        Keycap(id: "key", symbol: KeyNames.label(for: keyCode), name: nil, phase: phase)
    }

    static func caps(for chord: KeyChord) -> [Keycap] {
        modifierCaps(for: chord.modifierFlags, phase: .pressed) + [keyCap(keyCode: chord.keyCode, phase: .pressed)]
    }

    static let ghostCommand = Keycap(id: "command", symbol: "⌘", name: "cmd", phase: .ghost)
    static let ghostKey = Keycap(id: "key", symbol: "A", name: nil, phase: .ghost)
}

enum KeycapTint: Equatable {
    case neutral
    case rejected
    case accepted

    static let rejectedColor = Color(red: 0.94, green: 0.43, blue: 0.46)
    static let acceptedColor = Color(red: 0.30, green: 0.84, blue: 0.45)
}

/// A dark key whose face sits down when pressed, with a lip that lights in the verdict's colour.
private struct KeycapView: View {
    let cap: Keycap
    let tint: KeycapTint

    private static let height: CGFloat = 44
    private static let modifierWidth: CGFloat = 64
    private static let keyMinWidth: CGFloat = 38
    private static let radius: CGFloat = 8
    private static let keyTravel: CGFloat = 2

    private var isPressed: Bool { cap.phase == .pressed }

    var body: some View {
        label
            .frame(width: cap.isModifier ? Self.modifierWidth : nil, height: Self.height)
            .frame(minWidth: cap.isModifier ? nil : Self.keyMinWidth)
            .background { face }
            .shadow(color: .black.opacity(0.45), radius: 1.5, x: 0, y: 1)
            .opacity(isPressed ? 1 : 0.36)
            .animation(.spring(response: 0.3, dampingFraction: 0.8), value: isPressed)
            .accessibilityLabel(Text(verbatim: cap.name ?? cap.symbol))
    }

    private var face: some View {
        let shape = RoundedRectangle(cornerRadius: Self.radius, style: .continuous)
        return ZStack {
            shape.fill(Color(white: 0.07))
                .offset(y: Self.keyTravel)
            shape
                .fill(LinearGradient(colors: [Color(white: 0.225), Color(white: 0.155)], startPoint: .top, endPoint: .bottom))
                .overlay {
                    shape.strokeBorder(
                        LinearGradient(colors: [Color.white.opacity(0.09), Color.white.opacity(0)], startPoint: .top, endPoint: .center),
                        lineWidth: 1
                    )
                }
                .overlay(alignment: .bottom) {
                    Capsule()
                        .fill(lipColor)
                        .frame(height: 1.5)
                        .padding(.horizontal, 7)
                        .padding(.bottom, 2.5)
                }
                .offset(y: isPressed ? Self.keyTravel - 0.5 : 0)
        }
    }

    @ViewBuilder
    private var label: some View {
        if cap.isModifier {
            VStack(alignment: .trailing, spacing: 2) {
                Text(verbatim: cap.symbol)
                    .font(.system(size: 12, weight: .medium))
                Text(verbatim: cap.name ?? "")
                    .font(.system(size: 11, weight: .semibold))
            }
            .foregroundStyle(inkColor)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomTrailing)
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .offset(y: isPressed ? Self.keyTravel - 0.5 : 0)
        } else {
            Text(verbatim: cap.symbol)
                .font(.system(size: 14, weight: .semibold))
                .lineLimit(1)
                .fixedSize()
                .foregroundStyle(inkColor)
                .padding(.horizontal, 10)
                .id(cap.symbol)
                .transition(.scale(scale: 0.6).combined(with: .opacity))
                .offset(y: isPressed ? Self.keyTravel - 0.5 : 0)
        }
    }

    private var inkColor: Color {
        switch tint {
        case .neutral: Color.white.opacity(0.92)
        case .rejected: KeycapTint.rejectedColor
        case .accepted: KeycapTint.acceptedColor
        }
    }

    private var lipColor: Color {
        guard isPressed else { return Color.white.opacity(0.10) }
        return switch tint {
        case .neutral: Color.white.opacity(0.55)
        case .rejected: KeycapTint.rejectedColor.opacity(0.9)
        case .accepted: KeycapTint.acceptedColor.opacity(0.9)
        }
    }
}

// MARK: - Recording popover

struct ShortcutRecordingAction: Identifiable {
    let id: String
    let title: String
    let isDestructive: Bool
    let perform: () -> Void
}

private struct ShortcutRecordingPopoverContent: View {
    let session: ShortcutRecordingSession
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 12) {
            HStack(spacing: 6) {
                ForEach(session.caps) { cap in
                    KeycapView(cap: cap, tint: session.tint)
                        .transition(.scale(scale: 0.5).combined(with: .opacity))
                }
            }
            if let message = session.message {
                Text(verbatim: message)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(KeycapTint.rejectedColor)
                    .lineLimit(1)
                    .transition(.opacity)
            }
            if !session.actions.isEmpty {
                HStack(spacing: 8) {
                    ForEach(session.actions) { action in
                        Button(action: action.perform) {
                            Text(verbatim: action.title)
                                .font(.system(size: 12, weight: .semibold))
                                .lineLimit(1)
                                .padding(.horizontal, 12)
                                .frame(height: 26)
                        }
                        .buttonStyle(ShortcutRecordingActionPillStyle(isDestructive: action.isDestructive))
                    }
                }
            }
        }
        .padding(14)
        .fixedSize()
        .animation(reduceMotion ? .easeOut(duration: 0.12) : .spring(response: 0.3, dampingFraction: 0.8), value: session.caps)
        .animation(.easeOut(duration: 0.14), value: session.tint)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text("Press keys…"))
    }
}

private struct ShortcutRecordingActionPillStyle: ButtonStyle {
    let isDestructive: Bool

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(isDestructive ? KeycapTint.rejectedColor : Chrome.primaryText)
            .glassEffect(.regular.interactive(), in: Capsule(style: .continuous))
            .scaleEffect(configuration.isPressed ? 0.97 : 1)
            .contentShape(Capsule(style: .continuous))
    }
}

enum Haptics {
    static func perform(_ pattern: NSHapticFeedbackManager.FeedbackPattern) {
        NSHapticFeedbackManager.defaultPerformer.perform(pattern, performanceTime: .now)
    }
}

/// One recording, from the click on the control to the popover folding away. A chord is judged the
/// moment its key goes down and can be replaced while anything is held; it saves once every key is up.
@MainActor
@Observable
final class ShortcutRecordingSession {
    enum Verdict: Equatable {
        case listening
        case rejected
        case accepted
    }

    private(set) var caps: [Keycap] = [Keycap.ghostCommand]
    private(set) var verdict: Verdict = .listening
    private(set) var actions: [ShortcutRecordingAction] = []
    private(set) var message: String?

    @ObservationIgnored private var isActive = false
    @ObservationIgnored private var shortcut: AppShortcut?
    @ObservationIgnored private var onClose: (() -> Void)?
    @ObservationIgnored private var onCommit: ((KeyChord) -> Void)?
    @ObservationIgnored private var heldModifiers: NSEvent.ModifierFlags = []
    @ObservationIgnored private var heldKeyCode: Int?
    @ObservationIgnored private var latched: KeyChord?
    @ObservationIgnored private var isFinishing = false
    @ObservationIgnored private var monitor: Any?
    @ObservationIgnored private var resignObserver: NSObjectProtocol?
    @ObservationIgnored private var finishTask: Task<Void, Never>?

    var tint: KeycapTint {
        switch verdict {
        case .listening: .neutral
        case .rejected: .rejected
        case .accepted: .accepted
        }
    }

    func begin(
        shortcut: AppShortcut,
        actions: [ShortcutRecordingAction],
        onClose: @escaping () -> Void,
        onCommit: @escaping (KeyChord) -> Void
    ) {
        if isActive { close() }
        self.shortcut = shortcut
        self.actions = actions
        self.onClose = onClose
        self.onCommit = onCommit
        heldKeyCode = nil
        latched = nil
        isFinishing = false
        verdict = .listening
        message = nil
        heldModifiers = NSEvent.modifierFlags.intersection(KeyChord.allowedModifiers)
        refreshCaps()
        isActive = true

        monitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .keyUp, .flagsChanged]) { [weak self] event in
            guard let self, self.isActive else { return event }
            switch event.type {
            case .flagsChanged: self.handleFlagsChanged(event)
            case .keyDown: self.handleKeyDown(event)
            case .keyUp: self.handleKeyUp(event)
            default: return event
            }
            return nil
        }
        resignObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didResignActiveNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.cancel() }
        }
    }

    func cancel() {
        guard isActive else { return }
        close()
    }

    private func close() {
        finishTask?.cancel()
        finishTask = nil
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
        if let resignObserver { NotificationCenter.default.removeObserver(resignObserver) }
        resignObserver = nil
        isActive = false
        isFinishing = false
        let onClose = self.onClose
        self.onClose = nil
        onClose?()
    }

    private func handleFlagsChanged(_ event: NSEvent) {
        let flags = event.modifierFlags.intersection(KeyChord.allowedModifiers)
        let added = !flags.subtracting(heldModifiers).isEmpty
        heldModifiers = flags
        guard !isFinishing else { return }
        if added { Haptics.perform(.alignment) }
        if flags.isEmpty, heldKeyCode == nil {
            letGo()
        } else {
            refreshCaps()
        }
    }

    private func handleKeyDown(_ event: NSEvent) {
        guard !event.isARepeat else { return }
        let keyCode = Int(event.keyCode)
        let flags = event.modifierFlags.intersection(KeyChord.allowedModifiers)
        if keyCode == 53, flags.isEmpty {
            cancel()
            return
        }
        guard !isFinishing else { return }
        heldKeyCode = keyCode
        heldModifiers = flags
        judge(KeyChord(keyCode: keyCode, modifiers: flags))
    }

    private func handleKeyUp(_ event: NSEvent) {
        guard heldKeyCode == Int(event.keyCode) else { return }
        heldKeyCode = nil
        guard !isFinishing else { return }
        if heldModifiers.isEmpty {
            letGo()
        } else {
            refreshCaps()
        }
    }

    private func judge(_ chord: KeyChord) {
        latched = chord
        if let shortcut, let refusal = ShortcutStore.shared.refusal(for: chord, replacing: shortcut) {
            verdict = .rejected
            message = refusal
            Haptics.perform(.generic)
        } else {
            verdict = .accepted
            message = nil
            Haptics.perform(.levelChange)
        }
        refreshCaps()
    }

    private func letGo() {
        guard let latched else {
            refreshCaps()
            return
        }
        switch verdict {
        case .accepted:
            isFinishing = true
            refreshCaps()
            finishTask = Task { @MainActor [weak self] in
                try? await Task.sleep(for: .milliseconds(300))
                guard let self, !Task.isCancelled, self.isActive else { return }
                self.onCommit?(latched)
                self.close()
            }
        case .rejected:
            self.latched = nil
            verdict = .listening
            refreshCaps()
        case .listening:
            refreshCaps()
        }
    }

    private func refreshCaps() {
        let next: [Keycap]
        if let latched, verdict != .listening {
            next = Keycap.caps(for: latched)
        } else {
            let modifiers = Keycap.modifierCaps(for: heldModifiers, phase: .pressed)
            if modifiers.isEmpty {
                next = [Keycap.ghostCommand]
            } else if let heldKeyCode {
                next = modifiers + [Keycap.keyCap(keyCode: heldKeyCode, phase: .pressed)]
            } else {
                next = modifiers + [Keycap.ghostKey]
            }
        }
        if next != caps { caps = next }
    }
}
