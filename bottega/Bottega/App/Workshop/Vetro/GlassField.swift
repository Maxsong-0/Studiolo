// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

extension View {
    /// For a one-line `TextField` on glass: a capsule filled as a card is, with the outline every control has,
    /// brighter while it is typed in. The system's rounded field is a dark box on dark glass, and nothing in this
    /// window sits on black (T-245).
    func glassField(height: CGFloat = 28) -> some View { modifier(GlassField(height: height)) }

    /// What makes a field on glass one, whatever its shape: the capsule of a one-line field, the box an answer of
    /// several lines grows into. Without the outline a field is only its fill, and that goes when the glass is
    /// clear, leaving nothing to say where to type.
    func glassFieldBox(in shape: some InsettableShape, isTypedIn: Bool) -> some View {
        modifier(GlassFieldBox(shape: shape, isTypedIn: isTypedIn))
    }
}

private struct GlassField: ViewModifier {
    @FocusState private var isTypedIn: Bool
    let height: CGFloat

    func body(content: Content) -> some View {
        content
            .textFieldStyle(.plain)
            .focused($isTypedIn)
            .padding(.horizontal, height * 0.4)
            .frame(height: height)
            .glassFieldBox(in: Capsule(), isTypedIn: isTypedIn)
    }
}

private struct GlassFieldBox<S: InsettableShape>: ViewModifier {
    @Environment(Look.self) private var look
    @Environment(\.colorSchemeContrast) private var contrast
    let shape: S
    let isTypedIn: Bool

    func body(content: Content) -> some View {
        content
            .background(look.surface, in: shape)
            .vetroEdge(in: shape)
            .overlay {
                if isTypedIn {
                    shape.strokeBorder(look.focusEdge(increasedContrast: contrast == .increased), lineWidth: 1.5)
                }
            }
            .contentShape(shape)
            .animation(.easeOut(duration: 0.15), value: isTypedIn)
    }
}
