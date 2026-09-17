import SwiftUI

/// An info mark that opens its explainer on hover and keeps it open while the
/// pointer is over the popover, closing a beat after the pointer leaves both.
struct InfoHoverButton<Content: View>: View {
    let help: String
    var width: CGFloat = 360
    @ViewBuilder let content: () -> Content

    @State private var isHovering = false
    @State private var isOverPopover = false
    @State private var isPresented = false
    @State private var closeTask: Task<Void, Never>?

    var body: some View {
        Image(systemName: "info.circle")
            .font(.system(size: 12, weight: .medium))
            .foregroundStyle(Chrome.secondaryText.opacity(isHovering ? 1 : 0.6))
            .frame(width: 20, height: 20)
            .contentShape(.rect)
            .focusable(false)
            .help(help)
            .accessibilityLabel(Text(help))
            .accessibilityAddTraits(.isButton)
            .onHover { over in
                isHovering = over
                if over {
                    closeTask?.cancel()
                    isPresented = true
                } else {
                    scheduleClose()
                }
            }
            .onTapGesture { isPresented.toggle() }
            .popover(isPresented: $isPresented, arrowEdge: .bottom) {
                content()
                    .frame(width: width)
                    .onHover { over in
                        isOverPopover = over
                        if over {
                            closeTask?.cancel()
                        } else {
                            scheduleClose()
                        }
                    }
            }
    }

    private func scheduleClose() {
        closeTask?.cancel()
        closeTask = Task {
            try? await Task.sleep(for: .milliseconds(320))
            guard !Task.isCancelled, !isHovering, !isOverPopover else { return }
            isPresented = false
        }
    }
}
