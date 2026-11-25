// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// Something that comes and goes over the top of a page, without moving what is under it: a button must not slide
/// away from under the pointer.
struct FloatingTop<Content: View, Over: View>: View {
    /// Where nothing on the page signs or sends anything, it makes room instead, so that its head is not covered.
    let makesRoom: Bool
    @ViewBuilder let content: Content
    @ViewBuilder let over: Over
    @State private var height: CGFloat = 0

    var body: some View {
        content
            .padding(.top, makesRoom ? height : 0)
            .overlay(alignment: .top) {
                VStack(spacing: 0) { over }
                    .frame(maxWidth: .infinity)
                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { height = $0 }
            }
    }
}

enum FloatingTopRoom {
    /// How far down a page that does not make room starts, so that one line over its top covers nothing while it is
    /// scrolled to its start. Not made and taken away as needed: SwiftUI moves the content along with its insets.
    static let reserved: CGFloat = 52
}
