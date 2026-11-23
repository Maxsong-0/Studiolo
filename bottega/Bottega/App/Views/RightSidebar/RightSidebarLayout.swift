// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// The right sidebar: a column pinned to the window's trailing edge, the mirror of the
/// thread list on the left. It holds one page at a time — the Crew deck, the thread's
/// changes, or the tasks — picked by the tabs across its top and by the switches in the
/// chat's chrome row. No page is the column closed. (The terminal is not one of them: it
/// is a drawer at the foot of the chat, where a shell belongs.)
///
/// The column is the window's, not a thread's: switching threads keeps the page you are
/// on and fills it with the new thread's deck, shell and diff, the way the list on the
/// left keeps its scroll. Its width is dragged from its leading edge and kept across
/// launches; dragged past the collapse point it closes, like the left sidebar's.
@MainActor
@Observable
final class RightSidebarLayout {
    enum Tab: String, CaseIterable, Identifiable, Sendable {
        case crew
        case changes
        case task

        var id: String { rawValue }

        var title: String {
            switch self {
            case .crew: "Crew"
            case .changes: "Changes"
            case .task: "Tasks"
            }
        }

        /// The symbol the tab wears; the Crew tab wears the anchor asset instead.
        var symbol: String {
            switch self {
            case .crew: "person.2"
            case .changes: "plusminus"
            case .task: "checklist"
            }
        }

        var asset: String? { self == .crew ? "crew-mark" : nil }

        /// The chord that opens this page, where it has one.
        var shortcut: AppShortcut? {
            switch self {
            case .crew: .toggleCrewPanel
            case .changes: .toggleChanges
            case .task: nil
            }
        }
    }

    static let minimumWidth: CGFloat = 340
    static let maximumWidth: CGFloat = 760
    static let collapseThreshold: CGFloat = 240
    static let defaultWidth: CGFloat = 420

    private enum Key {
        static let width = "rightSidebarWidth"
        static let tab = "rightSidebarTab"
    }

    private(set) var width: CGFloat
    /// The page on show; nil is closed.
    private(set) var tab: Tab?
    /// True while the column's edge is held: the chat follows the width with no spring
    /// until it is let go.
    private(set) var isDragging = false
    /// True for the run of the open or close slide, so the chat treats it as a pane
    /// reshape rather than springing every row.
    private(set) var isSliding = false
    @ObservationIgnored private var slideTask: Task<Void, Never>?

    @ObservationIgnored private var anchorWidth: CGFloat = 0
    @ObservationIgnored private var restingWidth: CGFloat

    /// The spring of Chrome.panelSlide is visually settled by then.
    private static var slideRun: Int { NSWorkspace.shared.accessibilityDisplayShouldReduceMotion ? 220 : 420 }

    init() {
        let defaults = UserDefaults.standard
        let stored = (defaults.object(forKey: Key.width) as? Double).map { CGFloat($0) } ?? Self.defaultWidth
        restingWidth = min(Self.maximumWidth, max(Self.minimumWidth, stored))
        width = restingWidth
        // The page is remembered, so a window that closed on the deck opens on it.
        tab = (defaults.object(forKey: Key.tab) as? String).flatMap(Tab.init(rawValue:))
    }

    var isVisible: Bool { tab != nil }

    var renderedWidth: CGFloat { isVisible ? width : 0 }

    func open(_ tab: Tab) {
        guard self.tab != tab else { return }
        let opening = self.tab == nil
        if opening { width = restingWidth }
        beginSlide()
        withAnimation(Chrome.panelSlide) { self.tab = tab }
        persist()
    }

    func close() {
        guard tab != nil else { return }
        beginSlide()
        withAnimation(Chrome.panelSlide) { tab = nil }
        persist()
    }

    /// The switches and the shortcuts: the page they name, or closed if it is already on.
    func toggle(_ tab: Tab) {
        if self.tab == tab { close() } else { open(tab) }
    }

    func beginDrag() {
        slideTask?.cancel()
        isSliding = false
        isDragging = true
        if isVisible {
            anchorWidth = width
            return
        }
        anchorWidth = 0
        var transaction = Transaction()
        transaction.disablesAnimations = true
        withTransaction(transaction) {
            width = 0
            tab = .crew
        }
    }

    /// The handle is on the column's leading edge, so the pointer travelling left — a
    /// negative translation — is the column growing.
    func drag(by translation: CGFloat) {
        width = min(Self.maximumWidth, max(0, anchorWidth - translation))
    }

    func endDrag() {
        if width < Self.collapseThreshold {
            beginSlide()
            withAnimation(Chrome.panelSlide) { tab = nil }
            width = restingWidth
        } else {
            let settled = max(Self.minimumWidth, width)
            restingWidth = settled
            withAnimation(Chrome.panelSlide) { width = settled }
        }
        persist()
        isDragging = false
    }

    private func beginSlide() {
        isSliding = true
        slideTask?.cancel()
        slideTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .milliseconds(Self.slideRun))
            guard !Task.isCancelled, let self else { return }
            isSliding = false
        }
    }

    private func persist() {
        let defaults = UserDefaults.standard
        defaults.set(Double(restingWidth), forKey: Key.width)
        if let tab {
            defaults.set(tab.rawValue, forKey: Key.tab)
        } else {
            defaults.removeObject(forKey: Key.tab)
        }
    }
}
