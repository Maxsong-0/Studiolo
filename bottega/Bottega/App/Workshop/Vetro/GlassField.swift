// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

extension View {
    /// For a one-line `TextField` on glass: a capsule a shade lighter than the pane, with the outline every control
    /// has, brighter while it is typed in. The system's rounded field is a dark box on dark glass, and nothing in
    /// this window sits on black (T-245).
    func glassField(height: CGFloat = 28) -> some View { modifier(GlassField(height: height)) }
}

private struct GlassField: ViewModifier {
    @Environment(Look.self) private var look
    @FocusState private var isTypedIn: Bool
    let height: CGFloat

    func body(content: Content) -> some View {
        content
            .textFieldStyle(.plain)
            .focused($isTypedIn)
            .padding(.horizontal, height * 0.4)
            .frame(height: height)
            .background(look.track, in: .capsule)
            .vetroEdge(in: Capsule())
            .overlay { if isTypedIn { Capsule().strokeBorder(look.focusEdge, lineWidth: 1.5) } }
            .contentShape(.capsule)
            .animation(.easeOut(duration: 0.15), value: isTypedIn)
    }
}
