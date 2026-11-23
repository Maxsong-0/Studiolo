// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

extension View {
    /// What every SwiftUI-presented tree (popover, sheet, hosted view) gets so its
    /// buttons match the window's capsules: the tree hangs off the window's own,
    /// so it needs the window root's capsule shape and tint restated here.
    func presentedChrome() -> some View {
        self
            .buttonBorderShape(.capsule)
            .tint(ThemeManager.spec.accent)
    }
}

extension NSPopover {
    /// Installs SwiftUI content at a fixed size and makes that size the only
    /// one the panel ever has. A hosting controller normally publishes its
    /// own `preferredContentSize` a moment after `show(relativeTo:)`, and
    /// AppKit applies it by resizing the shown panel from its top edge, so
    /// the arrow ends up floating well above the anchor. Sizing the hosting
    /// controller off (`sizingOptions = []`) leaves `contentSize` in charge,
    /// and framing the content to it keeps SwiftUI filling the panel.
    ///
    /// The panel hosts a SwiftUI tree of its own, outside the window's, so it
    /// is given what the window root gives every view: capsule buttons and the
    /// theme's tint. A Save button in it is then the same capsule as one in
    /// Settings, not the stock rounded rectangle.
    @MainActor
    func setFixedContent<Content: View>(_ content: Content, size: NSSize) {
        // Top-aligned: content that turns out taller than measured loses its tail behind
        // the panel's edge, never its header.
        let root = content
            .frame(width: size.width, height: size.height, alignment: .top)
            .clipped()
            .presentedChrome()
        let host = NSHostingController(rootView: root)
        host.sizingOptions = []
        contentViewController = host
        contentSize = size
    }
}

/// A popover's scrolling body, as tall as its content up to `maxHeight` and scrolling
/// past that. A popover opens at its content's ideal size, and a scroll view offered
/// nothing collapses to a few rows; a fixed ideal height opens short content on empty
/// space and long content cut off. So the content's own height sets the frame, and
/// never past the cap.
///
/// The height read as the content lays out (`onGeometryChange`) lands one pass after
/// the popover has been sized, and AppKit animates the panel to the new size from its
/// top edge: the popover reads as sliding in from above. Given its `width`, the body
/// lays the content out once before the popover opens, so the first size is the final
/// one; the geometry read then only follows changes made while it is open.
struct PopoverScroll<Content: View>: View {
    var maxHeight: CGFloat = 460
    /// What the popover opens at before the content has laid out.
    var minHeight: CGFloat = 80
    let content: () -> Content
    @State private var contentHeight: CGFloat
    /// Whether the height was settled before the popover opened. A pre-measured body
    /// keeps that height: the panel around it was sized to it once and never resizes
    /// (see `NSPopover.setFixedContent`), so a later, taller reading only pushed the
    /// body past the panel and cut the header off; the scroll takes any difference.
    private let isPremeasured: Bool

    @MainActor
    init(maxHeight: CGFloat = 460, minHeight: CGFloat = 80, width: CGFloat? = nil, @ViewBuilder content: @escaping () -> Content) {
        self.maxHeight = maxHeight
        self.minHeight = minHeight
        self.content = content
        isPremeasured = width != nil
        _contentHeight = State(initialValue: width.map { Self.measuredHeight(of: content(), width: $0) } ?? 0)
    }

    /// The height `content` takes at `width`: one layout of it off screen, the same
    /// one the popover would run to size itself.
    @MainActor
    static func measuredHeight(of content: some View, width: CGFloat) -> CGFloat {
        NSHostingView(rootView: content.frame(width: width)).fittingSize.height.rounded(.up)
    }

    var body: some View {
        ScrollView {
            content()
                .onGeometryChange(for: CGFloat.self, of: { $0.size.height }) { height in
                    if !isPremeasured { contentHeight = height }
                }
        }
        .scrollBounceBehavior(.basedOnSize)
        .frame(height: min(max(contentHeight, minHeight), maxHeight))
    }
}

/// The native popover every chrome and composer button opens, in place of a pull-down menu.
struct PopoverMenu<Content: View>: View {
    var maxHeight: CGFloat = 460
    /// Forces the popover open at this height. Needed because a ScrollView inside
    /// a popover otherwise collapses to a few rows and maxHeight never engages.
    var idealHeight: CGFloat?
    @ViewBuilder var content: Content

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                content
            }
            .padding(6)
            .frame(minWidth: 210, maxWidth: 360, alignment: .leading)
        }
        .scrollBounceBehavior(.basedOnSize)
        .scrollIndicators(.automatic)
        .frame(idealHeight: idealHeight, maxHeight: maxHeight)
        .presentedChrome()
    }
}

struct PopoverSectionHeader: View {
    let title: String

    init(_ title: String) {
        self.title = title
    }

    var body: some View {
        Text(verbatim: title)
            .zoomedFont(size: 11, weight: .semibold)
            .foregroundStyle(Chrome.secondaryText)
            .padding(.horizontal, 8)
            .padding(.top, 6)
            .padding(.bottom, 3)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct PopoverNote: View {
    let text: String

    init(_ text: String) {
        self.text = text
    }

    var body: some View {
        Text(verbatim: text)
            .zoomedFont(size: 12)
            .foregroundStyle(Chrome.secondaryText)
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct PopoverDivider: View {
    var body: some View {
        Divider()
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
    }
}

/// The close closure in a box with an identity. A closure can never compare equal to
/// itself, so an environment value holding one bare tells SwiftUI that every reader of
/// the environment is out of date on every update; a box compares by reference and only
/// the popover's own host ever makes a new one.
final class PopoverCloseBox: Equatable {
    let close: @MainActor () -> Void

    init(_ close: @escaping @MainActor () -> Void) {
        self.close = close
    }

    static func == (lhs: PopoverCloseBox, rhs: PopoverCloseBox) -> Bool {
        lhs === rhs
    }
}

/// How a popover hosted by AppKit closes. SwiftUI's own popovers answer `dismiss`; one shown
/// through an `NSPopover` sets this instead, and every row in it closes through it.
extension EnvironmentValues {
    @Entry var closePopoverBox: PopoverCloseBox?

    /// Set and read as a plain closure; kept in the box above, which is what the rows
    /// actually read, so they are not invalidated by every unrelated environment change.
    var closePopover: (@MainActor () -> Void)? {
        get { closePopoverBox?.close }
        set { closePopoverBox = newValue.map(PopoverCloseBox.init) }
    }
}

/// One row. Pass `isChecked` (true or false) for choice lists so every row keeps the checkmark column.
struct PopoverItem: View {
    let title: String
    /// A styled title, such as a bold verb before a plain label, in place of the plain one.
    var titleText: Text?
    var detail: String?
    var symbol: String?
    var image: NSImage?
    var asset: String?
    /// A custom 16pt leading view (a colour swatch, say) in place of an icon.
    var leading: AnyView?
    var isChecked: Bool?
    var isEnabled = true
    var isDestructive = false
    /// What the row says once tapped ("Copied"), in place with a checkmark, before the
    /// popover closes on its own. The action runs at once, so it must not present anything.
    var confirmation: String?
    let action: @MainActor () -> Void

    @Environment(\.dismiss) private var dismiss
    @Environment(\.closePopoverBox) private var closePopoverBox
    @Environment(\.chatZoom) private var zoom
    @State private var isHovering = false
    @State private var isConfirmed = false

    /// How long the confirmation shows before the popover closes.
    private static let confirmationHold: Duration = .milliseconds(650)

    init(
        _ title: String,
        detail: String? = nil,
        symbol: String? = nil,
        image: NSImage? = nil,
        asset: String? = nil,
        leading: AnyView? = nil,
        isChecked: Bool? = nil,
        isEnabled: Bool = true,
        isDestructive: Bool = false,
        confirmation: String? = nil,
        action: @escaping @MainActor () -> Void
    ) {
        self.title = title
        self.detail = detail
        self.symbol = symbol
        self.image = image
        self.asset = asset
        self.leading = leading
        self.isChecked = isChecked
        self.isEnabled = isEnabled
        self.isDestructive = isDestructive
        self.confirmation = confirmation
        self.action = action
    }

    /// A row whose title is styled text; `title` names it for accessibility.
    init(
        _ title: String,
        text: Text,
        symbol: String? = nil,
        image: NSImage? = nil,
        isChecked: Bool? = nil,
        action: @escaping @MainActor () -> Void
    ) {
        self.title = title
        self.titleText = text
        self.symbol = symbol
        self.image = image
        self.isChecked = isChecked
        self.action = action
    }

    var body: some View {
        Button {
            if confirmation != nil, !isConfirmed {
                // The row answers in place first: the deed is done, the words say so, and
                // the popover goes a beat later once the checkmark has been seen.
                action()
                withAnimation(.snappy(duration: 0.22)) { isConfirmed = true }
                Task { @MainActor in
                    try? await Task.sleep(for: Self.confirmationHold)
                    if let close = closePopoverBox?.close { close() } else { dismiss() }
                }
                return
            }
            guard !isConfirmed else { return }
            if let close = closePopoverBox?.close { close() } else { dismiss() }
            // Runs after the popover closes, so an action that presents a sheet or alert is not swallowed.
            Task { @MainActor in action() }
        } label: {
            HStack(spacing: 8) {
                // The leading columns grow with the row's type (see `zoom`).
                let icon = ChatZoom.scaled(16, by: zoom)
                if let isChecked {
                    Image(systemName: "checkmark")
                        .zoomedFont(size: 11, weight: .semibold)
                        .opacity(isChecked ? 1 : 0)
                        .frame(width: ChatZoom.scaled(14, by: zoom))
                }
                if let leading {
                    leading
                        .frame(width: icon, height: icon)
                } else if let asset {
                    Image(asset)
                        .resizable()
                        .aspectRatio(contentMode: .fit)
                        .frame(width: icon, height: icon)
                } else if let image {
                    Image(nsImage: image)
                        .resizable()
                        .interpolation(.high)
                        .frame(width: icon, height: icon)
                } else if isConfirmed {
                    Image(systemName: "checkmark")
                        .zoomedFont(size: 12, weight: .semibold)
                        .foregroundStyle(Chrome.success)
                        .frame(width: icon)
                        .transition(.scale(scale: 0.6).combined(with: .opacity))
                } else if let symbol {
                    Image(systemName: symbol)
                        .zoomedFont(size: 12)
                        .frame(width: icon)
                        .transition(.scale(scale: 0.6).combined(with: .opacity))
                }
                (isConfirmed ? Text(verbatim: confirmation ?? title) : (titleText ?? Text(verbatim: title)))
                    .zoomedFont(size: 13)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .contentTransition(.numericText())
                Spacer(minLength: 16)
                if let detail {
                    Text(verbatim: detail)
                        .zoomedFont(size: 12)
                        .foregroundStyle(Chrome.secondaryText)
                        .lineLimit(1)
                }
            }
            .foregroundStyle(isDestructive ? Chrome.danger : Chrome.primaryText)
            .padding(.horizontal, 8)
            // A line of the row's type with room around it: taller with the window's text size.
            .frame(height: ChatZoom.scaled(26, by: zoom))
            .background {
                RoundedRectangle(cornerRadius: Chrome.menuRowCornerRadius, style: .continuous)
                    .fill(isHovering && isEnabled ? Chrome.overlay(0.1) : Color.clear)
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .disabled(!isEnabled)
        .opacity(isEnabled ? 1 : 0.4)
        .accessibilityLabel(Text(detail.map { "\(title), \($0)" } ?? title))
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
    }
}

// MARK: - Row actions

/// An action a sidebar row offers both from its ellipsis popover and its context menu.
struct RowAction: Identifiable {
    let title: String
    var symbol: String?
    var isDestructive = false
    var startsGroup = false
    /// What the popover row says in place once tapped ("Copied"); see `PopoverItem.confirmation`.
    var confirms: String?
    let action: @MainActor () -> Void

    var id: String { title }
}

struct RowActionItems: View {
    let actions: [RowAction]

    var body: some View {
        ForEach(actions) { item in
            if item.startsGroup { PopoverDivider() }
            PopoverItem(item.title, symbol: item.symbol, isDestructive: item.isDestructive, confirmation: item.confirms, action: item.action)
        }
    }
}

struct RowActionMenuButtons: View {
    let actions: [RowAction]

    var body: some View {
        ForEach(actions) { item in
            if item.startsGroup { Divider() }
            Button(role: item.isDestructive ? .destructive : nil) {
                item.action()
            } label: {
                if let symbol = item.symbol {
                    Label(item.title, systemImage: symbol)
                } else {
                    Text(item.title)
                }
            }
        }
    }
}

/// The ellipsis on a hovered sidebar row.
struct RowActionsButton: View {
    let actions: [RowAction]
    @Binding var isPresented: Bool

    var body: some View {
        Button {
            isPresented.toggle()
        } label: {
            RowAccessoryIcon("ellipsis")
        }
        .buttonStyle(.plain)
        .help("More")
        .popover(isPresented: $isPresented, arrowEdge: .bottom) {
            PopoverMenu { RowActionItems(actions: actions) }
        }
    }
}
