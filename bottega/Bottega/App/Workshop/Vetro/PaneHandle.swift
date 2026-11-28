// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// The strip along a side pane's inner edge. Dragging it makes the pane wider or narrower; dragged well past its
/// narrowest, the pane is put away (`SidePane`). Nothing is drawn: the pointer turning into the two arrows says it
/// is there, as on every Mac split view.
struct PaneHandle: View {
    @Environment(WorkshopModel.self) private var model
    let pane: SidePane
    /// How wide the pane is drawn, which is where a drag starts from.
    let width: Double
    /// How wide the window is, which is what decides how wide the pane may get.
    let window: Double
    /// The width when the drag began; the drag is measured from there, not from the last step.
    @State private var from: Double?

    var body: some View {
        Color.clear
            .frame(width: WindowMeasures.handle)
            .frame(maxHeight: .infinity)
            .contentShape(.rect)
            .pointerStyle(.columnResize)
            .gesture(
                DragGesture(minimumDistance: 1, coordinateSpace: .global)
                    .onChanged { drag in
                        let start = from ?? width
                        from = start
                        // The sidebar grows to the right, the panel to the left.
                        let moved = pane == .sidebar ? drag.translation.width : -drag.translation.width
                        model.drag(pane, to: start + moved, from: start, window: window)
                    }
                    .onEnded { _ in from = nil }
            )
            .accessibilityElement()
            .accessibilityLabel(pane == .sidebar ? "Sidebar width" : "Panel width")
            .accessibilityValue(Text(verbatim: "\(Int(width.rounded())) points"))
            .accessibilityAdjustableAction { direction in
                let step = direction == .increment ? Self.step : -Self.step
                model.drag(pane, to: width + step, from: width, window: window)
            }
    }

    /// What VoiceOver's up and down change it by.
    private static let step = 20.0
}
