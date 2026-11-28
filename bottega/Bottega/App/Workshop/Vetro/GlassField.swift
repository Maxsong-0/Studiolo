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
    /// `typeHere` puts the cursor in the field. The box is bigger than the text in it, and a click anywhere in
    /// the box, on the padding between the outline and the words too, is a click to type there (T-254).
    func glassFieldBox(in shape: some InsettableShape, isTypedIn: Bool, typeHere: @escaping () -> Void) -> some View {
        modifier(GlassFieldBox(shape: shape, isTypedIn: isTypedIn, typeHere: typeHere))
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
            .glassFieldBox(in: Capsule(), isTypedIn: isTypedIn) { isTypedIn = true }
    }
}

private struct GlassFieldBox<S: InsettableShape>: ViewModifier {
    @Environment(Look.self) private var look
    let shape: S
    let isTypedIn: Bool
    let typeHere: () -> Void

    func body(content: Content) -> some View {
        content
            .background(look.surface, in: shape)
            .vetroEdge(in: shape)
            .vetroRing(isTypedIn, in: shape)
            .contentShape(shape)
            // The field itself takes its own clicks; this gets the ones around it.
            .onTapGesture(perform: typeHere)
            .animation(.easeOut(duration: 0.15), value: isTypedIn)
    }
}
